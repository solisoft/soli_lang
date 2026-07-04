# PDF & Factur-X Generation

Render a PDF from a **JSON layout template** + a **JSON data** document, in-process — no headless browser, no wkhtmltopdf, no Node. Two builtins:

- `pdf_render(template, data, options?)` — an ordinary PDF.
- `pdf_facturx(template, data, xml, options?)` — a **PDF/A-3b Factur-X (EN 16931)** electronic invoice with the Cross-Industry-Invoice XML embedded (you supply the XML).
- `pdf_facturx_from_invoice(template, invoice, options?)` — the same PDF/A-3b Factur-X, but the CII XML **and** the visual totals/VAT breakdown are *generated* from a typed [invoice document](#typed-invoice-document) — no hand-written XML.

All three return the PDF as a **base64 string** (Soli has no bytes type). Save it with `file_write_base64(path, b64)`.

---

## Quickstart

```soli
let template = slurp("pdf/invoice.json")
let data     = slurp("pdf/data.json")

# Plain PDF
let pdf = pdf_render(template, data)
file_write_base64("out/invoice.pdf", pdf)

# Factur-X PDF/A-3b (embeds your EN 16931 CII XML)
let xml = slurp("pdf/factur-x.xml")
let fx  = pdf_facturx(template, data, xml, { "profile": "en16931", "title": "Invoice 42" })
file_write_base64("out/facturx.pdf", fx)

# Factur-X from a typed invoice — totals, VAT breakdown and CII XML are generated
let invoice = slurp("pdf/invoice.json")
let fx2 = pdf_facturx_from_invoice(template, invoice, { "profile": "en16931" })
file_write_base64("out/facturx.pdf", fx2)
```

---

## Functions

### pdf_render(template, data, options?)

Render a PDF from a JSON layout `template` and a JSON `data` document.

**Parameters:**
- `template` (String) — the layout template JSON.
- `data` (String) — the data document JSON (`{ "data": {...} }` or a bare object).
- `options` (Hash, optional) — see [Options](#options).

**Returns:** String — base64-encoded PDF bytes.

### pdf_response(template, data, options?)

Render **and** wrap as a ready HTTP response — return it straight from a controller action, no `file_write_base64` + redirect dance:

```soli
def download
  let tpl  = slurp("pdf/invoice.json")
  let data = Invoice.find(params["id"]).to_json()
  return pdf_response(tpl, data, { "filename": "invoice-" + params["id"] + ".pdf" })
end
```

**Parameters:** as `pdf_render`, plus the optional `filename` option — when set, adds `Content-Disposition: attachment; filename="…"` (otherwise the browser renders the PDF inline).

**Returns:** Hash — `{ "status": 200, "headers": { "Content-Type": "application/pdf", … }, "body_base64": … }`. The `body_base64` key is decoded to the binary body by the server (available to any handler, not just PDFs).

### pdf_facturx(template, data, xml, options?)

Render the visual PDF, then embed the caller-provided CII `xml` and apply PDF/A-3b + Factur-X conformance (embedded `factur-x.xml`, `/AF` + name tree, sRGB OutputIntent, PDF/A + Factur-X XMP). **The library embeds the XML; it does not generate it.**

**Parameters:**
- `template`, `data` (String) — as above.
- `xml` (String) — your EN 16931 CII XML.
- `options` (Hash, optional) — the render options below plus `profile`, `title`, `author`, `subject`.

**Returns:** String — base64-encoded PDF/A-3b bytes.

### pdf_facturx_from_invoice(template, invoice, options?)

Render the visual PDF **and** generate the EN 16931 CII XML from a single typed [invoice document](#typed-invoice-document), then embed it (PDF/A-3b + Factur-X conformance). Line totals, the VAT breakdown, and the grand/amount-due totals are *computed* from the line items, so the human-readable PDF and the machine-readable XML can never disagree — which is the whole point of Factur-X.

The invoice is mapped onto the template's `${...}` paths (`invoice.*`, `company.*` (seller), `customer.*` (buyer), `items[]`, `discounts[]`, `charges[]`, `total.*`, `infos.*`) — see [Typed invoice document](#typed-invoice-document) for the field list.

**Parameters:**
- `template` (String) — the layout template JSON, using the paths above.
- `invoice` (String) — the typed invoice JSON.
- `options` (Hash, optional) — the render options below plus `profile`, `title`, `author`, `subject`.

**Returns:** String — base64-encoded PDF/A-3b bytes.

### pdf_from_markdown(markdown, options?)

Render a designed PDF straight from a **Markdown** string — *write prose, get a
PDF*. Headings, paragraphs, **bold**/*italic*/`code`/[links](#)/~~strike~~,
ordered & unordered (nested) lists, tables, fenced code blocks, blockquotes,
rules and images all map onto the layout engine's elements. No template to
author.

```soli
let md  = slurp("reports/quarterly.md")
let pdf = pdf_from_markdown(md, { "font_dirs": ["font"] })
file_write_base64("quarterly.pdf", pdf)
```

**Parameters:**
- `markdown` (String) — the Markdown source (CommonMark + tables, strikethrough, task lists).
- `options` (Hash, optional) — every [Option](#options) from the table below (`font_dirs`, `sign`, `pdfa`, `password`, …) **plus** theme overrides: `fonts` (Array — the font family, default `["titillium"]`), `fontSize` (body size, default 11), `lineHeight` (default 1.45), `headingColor`, `textColor`, `linkColor`, `codeColor` (all hex, no `#`).

**Returns:** String — base64-encoded PDF bytes. Composes with everything else, so `pdf_from_markdown(md, { sign: {…} })` gives a **signed** document from Markdown.

### pdf_fill(pdf, data, options?)

**Fill an existing PDF's form fields** (AcroForm) from data — the "take a
government/enterprise form and fill it programmatically" workflow that the render
builtins (which *write* PDFs) can't do. Sets text fields, checkboxes/radios and
choice fields.

```soli
# `pdf` is an app-root relative path, or base64 PDF bytes.
let filled = pdf_fill("forms/w9.pdf", {
  "full_name": "Ada Lovelace",
  "email":     "ada@example.com",
  "agree":     true          # checkbox: true/"yes"/"on"/"1" checks it
}, { "flatten": true })
file_write_base64("w9-filled.pdf", filled)
```

**Parameters:**
- `pdf` (String) — the source PDF: an app-root relative **path** to an existing file, or **base64** PDF bytes.
- `data` (Hash) — `{ field_name => value }`. Values are stringified; a bool drives a checkbox/radio's on/off state.
- `options` (Hash, optional) — `flatten` (Bool, default `false`): when `true`, bakes the values into static appearances, marks the fields read-only, and turns off the interactive form (`NeedAppearances`). When `false`, values are set and `NeedAppearances` is on, so viewers render them but the fields stay editable.

**Returns:** String — base64-encoded filled PDF. Errors if the PDF has no AcroForm (no fillable fields). Field names must match the form's own field names.

### pdf_merge(pdfs) · pdf_pages(pdf, selection) · pdf_stamp(pdf, text, options?)

Operate on **existing** PDFs — the toolkit half of the engine.

```soli
# Concatenate a cover + generated report + terms into one document.
let doc = pdf_merge([ "cover.pdf", report_b64, "terms.pdf" ])

# Keep a subset of pages ("1-3,7" range string, or [1,3,7]).
let excerpt = pdf_pages(doc, "1-3")

# Stamp a diagonal watermark on every page (or a subset).
let draft = pdf_stamp(doc, "DRAFT", {
  "opacity": 0.2, "color": "cc2222", "rotation": 45, "size": 60
})
```

- **`pdf_merge(pdfs)`** — `pdfs` is an array of PDF sources (each a **path** or **base64** bytes); returns them concatenated in order. Inherited page attributes (size, resources) are inlined so pages keep their look.
- **`pdf_pages(pdf, selection)`** — keep a subset: a range string `"1-3,7,9-11"` or an array `[1,3,7]` (1-based). Kept pages stay in their original order.
- **`pdf_stamp(pdf, text, options?)`** — draw `text` onto pages. Options: `pages` (range string / array; default all), `x`/`y` (points from bottom-left; default centered), `size` (default 48), `color` (hex, default grey), `rotation` (degrees, default 45), `opacity` (0–1, default 0.25). Ideal for `DRAFT`/`PAID`/`CONFIDENTIAL` watermarks.

All three take `pdf` as a **path or base64** and return base64.

### pdf_extract_facturx(pdf) · pdf_attachments(pdf)

Read a **received** e-invoice — the inverse of `pdf_facturx*`. Closes the loop:
you can now *process* incoming Factur-X / ZUGFeRD / XRechnung invoices, not just
emit them.

```soli
# Pull the embedded EN 16931 XML out of an invoice you received.
let xml = pdf_extract_facturx("inbox/supplier-invoice.pdf")
if xml.present?
  let invoice = Xml.parse(xml)   # then read totals, VAT, line items…
end

# Or list every embedded file.
for file in pdf_attachments(pdf)
  print("#{file["name"]} — #{file["mime"]} (#{file["size"]} bytes)")
end
```

- **`pdf_extract_facturx(pdf)`** — returns the embedded invoice XML as a **String**, or **null** if the PDF carries none. Matches the standard attachment names (`factur-x.xml`, `zugferd-invoice.xml`, `xrechnung.xml`, …).
- **`pdf_attachments(pdf)`** — returns an array of `{ "name", "mime", "size", "base64" }`, one per embedded file (decode `base64` for the bytes). Reads the `/EmbeddedFiles` name tree, falling back to `/AF`.

### pdf_sign(pdf, options) · pdf_verify(pdf)

Sign an **existing** PDF, and verify signatures on one you received.

```soli
# Sign a PDF you already have (a merged pack, a filled form, a received doc).
let signed = pdf_sign("contract.pdf", {
  cert: slurp("certs/signer.pem"),
  key:  slurp("certs/signer.key"),
  reason: "Approved",
  appearance: { page: 1, x: 360, y: 40, width: 190, height: 64 }   # optional visible block
})

# Verify the signatures on an incoming PDF.
for sig in pdf_verify(received_pdf)
  print("#{sig["field"]}: valid=#{sig["valid"]} covers_document=#{sig["covers_document"]} by #{sig["signer"]}")
end
```

- **`pdf_sign(pdf, options)`** — the standalone sibling of the [`sign`](#digital-signatures-pades) render option: `pdf` is a **path or base64**, and `options` is the sign config directly (`cert`, `key`, `chain?`, `reason?`, `location?`, `name?`, `contact?`, `tsa?`, `appearance?`). Returns base64. Composes with the toolkit — `merge → sign`, `fill → sign`.
- **`pdf_verify(pdf)`** — returns an array of `{ field, valid, covers_document, signer, reason?, signed_at? }`, one per signature. **`valid`** means the CMS signature verifies against its embedded certificate **and** the document's ByteRange digest matches — i.e. the content is authentic and unmodified. It does **not** assert certificate *trust* (whether the issuer is a trusted CA). `covers_document` is true when the signature spans the whole file.

### Options

| Key | Type | Default | Meaning |
|---|---|---|---|
| `font_dirs` | Array<String> | `["font"]` | Directories to load fonts from. No fonts are bundled. |
| `fetch_images` | Bool | `true` | Fetch `http(s)` images (`false` = offline/deterministic). |
| `profile` | String | `en16931` | *(Factur-X)* Factur-X profile. |
| `title` / `author` / `subject` | String | — | Document metadata (PDF Info dictionary). Works for `pdf_render` too; the plain-render title defaults to `"invoice"` when unset. |
| `stationery` | String | — | Path (app-root relative) to a **letterhead PDF** drawn beneath every page's content. Page 1 uses the letterhead's first page; later pages use its second page when present, else the first. A missing file is an error. The letterhead is scaled to the page size, and a template `background` fill paints over it. |
| `attachments` | Array | — | Files embedded into the reader's attachments panel: `[{ "path": "exports/data.csv", "name"?, "mime"? }]` (paths app-root relative; missing file = error; MIME guessed from the extension when omitted). Composes with Factur-X — `factur-x.xml` and your attachments coexist in the name tree and `/AF`. |
| `password` / `owner_password` | String | — | Password-protect the PDF (**AES-128**). `password` is required to open the document; `owner_password` lifts restrictions (defaults to `password`). **Incompatible with `pdf_facturx*`** — PDF/A forbids encryption. |
| `permissions` | Array | all | With a password set, the actions the user password permits: any of `["print", "copy", "modify", "annotate"]`. Empty (default) allows everything — a pure open-password. |
| `pdfa` | Bool | `false` | *(pdf_render / pdf_response)* Emit **PDF/A-3b** (archival conformance: sRGB OutputIntent, XMP `pdfaid` metadata, PDF 1.7) without any Factur-X payload — for legal-archiving mandates on documents that aren't invoices. Incompatible with `password` (PDF/A forbids encryption); `pdf_facturx*` reject it (they are already PDF/A). **Composes with a `tagged` template** — the output then declares PDF/A-3b *and* PDF/UA-1 (accessible + archival). Attachments compose. |
| `sign` | Hash | — | **Digitally sign** the PDF (PAdES) — see [Digital signatures](#digital-signatures-pades). `{ "cert", "key", "chain"?, "reason"?, "location"?, "name"?, "contact"? }`. Works on `pdf_render`, `pdf_response`, and `pdf_facturx*` (signed e-invoices). Incompatible with `password` (a signed PDF must not be encrypted). |

---

## Digital signatures (PAdES)

Pass a `sign` option to **cryptographically sign** the rendered PDF with a
detached CMS signature (PAdES baseline, `ETSI.CAdES.detached`). A reader (Adobe
Acrobat, Okular, `pdfsig`) can then confirm *who* issued the document and that it
**hasn't been modified since**. The signature is built in-process — no external
signing service — and layers on top of every other feature, including Factur-X,
so you can emit an invoice that is **archival (PDF/A-3b), machine-readable
(EN 16931 CII XML) and signed** in a single call.

```soli
# Sign a plain PDF.
pdf = pdf_render(template, data, {
  sign: {
    cert: slurp("config/certs/signer.pem"),   # signer certificate (PEM or path)
    key:  slurp("config/certs/signer.key"),   # private key (PEM or path)
    reason:   "Invoice issued",
    location: "Paris, FR",
    name:     "ACME SARL",
    contact:  "billing@acme.fr"
  }
})

# Sign a Factur-X e-invoice — the flagship: archival + machine-readable + signed.
pdf = pdf_facturx_from_invoice(template, invoice, {
  sign: { cert: signer_pem, key: signer_key }
})
```

**`sign` keys**

| Key | Required | Meaning |
|---|---|---|
| `cert` | ✓ | The signer's X.509 certificate. An inline PEM string, or an app-root relative path to a PEM/DER file. |
| `key` | ✓ | The private key. **RSA** (PKCS#1 or PKCS#8) or **EC P-256** (PKCS#8) PEM/DER. Inline string or path. |
| `chain` | — | Array of intermediate-certificate PEMs to embed so verifiers can build the trust path. |
| `tsa` | — | URL of an [RFC 3161](https://www.rfc-editor.org/rfc/rfc3161) Time-Stamp Authority. When set, the signature is timestamped (PAdES-**B-T**) — see below. Requires network access at sign time. |
| `appearance` | — | Draw a **visible** signature block (name, "Digitally signed", date, reason, location): `{ "page"?: 1, "x"?, "y"?, "width"?, "height"? }` in points from the page's bottom-left. Omit for an invisible-but-valid signature. |
| `reason` / `location` / `name` / `contact` | — | Human-facing metadata shown in the reader's signature panel. |

**What it produces.** A **PAdES-B-B** signature: SHA-256 digest, RSA or ECDSA
(P-256), with the standard signed attributes (content-type, message-digest,
signing-time, and the ESS `signing-certificate-v2` that binds the signature to
this exact certificate). The signature covers the whole document except its own
signature value, so any later edit invalidates it.

**Trusted timestamp (PAdES-B-T).** Add a `tsa` URL to have the signature
timestamped by a Time-Stamp Authority: the signature value is hashed and sent to
the TSA (`application/timestamp-query`), and the returned RFC 3161 token is
embedded as an unsigned attribute — independent proof the signature existed at a
given time, resilient to the signer's certificate later expiring.

```soli
pdf = pdf_render(template, data, {
  sign: {
    cert: signer_pem, key: signer_key,
    tsa: "http://timestamp.digicert.com"   # any RFC 3161 TSA
  }
})
```

**Key handling.** Read the certificate and key from files or env — never from
request data — and keep the private key out of source control. Soli reads the
material you pass and never logs it.

**Notes & limits.**

- **Incompatible with `password`** — a signed PDF must not be AES-encrypted; set one or the other.
- The signer certificate must chain to a root the *verifier* trusts for a "trusted" badge; a self-signed cert still verifies as *valid + unmodified*, just untrusted.
- A single signature. Long-term-validation (LTV) data — embedded OCSP/CRL responses for offline verification years later — is a planned follow-up.

---

## Storing a PDF on a model

The builtins return base64, so persisting a generated PDF is a question of *where the bytes live* — three options, lightest commitment last.

### Uploader (recommended)

Declare an [uploader](models.md#uploaders) and attach the generated bytes — Soli stores the blob in SoliDB, keeps only its id on the document, and serves it back. No HTTP upload is involved; `attach_<field>` just needs a file hash:

```soli
class Invoice < Model
  uploader("pdf", {
    "multiple":      false,
    "content_types": ["application/pdf"],
    "max_size":      10_000_000,
    "collection":    "invoice_pdfs"
  })

  # Render the Factur-X for this record and file it as the `pdf` attachment.
  def store_pdf(template, invoice_json)
    let pdf = pdf_facturx_from_invoice(template, invoice_json, { "profile": "en16931" })
    this.attach_pdf({
      "data":         pdf,                          # pass base64 as-is — stored as raw bytes (decoded automatically)
      "filename":     "invoice-#{this.number}.pdf",
      "content_type": "application/pdf",
      "size":         Base64.decode(pdf).length()   # raw byte count, for the max_size cap
    })
  end
end
```

Add `uploads("invoices")` to `config/routes.sl` and the file is served at `/invoices/:id/pdf`; `this.pdf_url()` returns the link. `detach_pdf()` removes it, and deleting the record cleans up the blob.

### Direct blob storage

Skip the DSL and keep the blob id yourself:

```soli
let client  = Solidb(getenv("SOLIDB_HOST"), getenv("SOLIDB_DATABASE"))
let blob_id = solidb_store_blob(client, "invoice_pdfs", pdf,
                                "invoice.pdf", "application/pdf")
invoice.update({ "pdf_blob_id": blob_id })

# read it back (returns base64)
let stored = solidb_get_blob(client, "invoice_pdfs", invoice.pdf_blob_id)
```

### Inline field

Simplest — store the base64 string straight on the document:

```soli
invoice.update({ "pdf": pdf })
```

Fine for small, occasional PDFs. Avoid it when the PDFs are large or you query that collection often: every read of the record carries the full base64 (a 135 KB PDF ≈ 180 KB of text). Blob storage keeps the document lean and the bytes out of band.

---

## Template reference

A template has five top-level keys:

```json
{
  "fonts":   ["titillium"],
  "options": { "header_height": 0, "watermark": { "text": "PAID" } },
  "header":  [],
  "footer":  [],
  "content": []
}
```

| key | type | meaning |
|---|---|---|
| `fonts` | `string[]` | Families to load; the first is the primary text face, the rest are fallbacks. |
| `options` | object | Document options — see below. |
| `header` | element[] | Drawn in the reserved top band on every page. Elements interpolate `${...}`, may use `#PAGE#`/`#PAGES#`, and **data-bound elements** (`table`, `repeat`, `chart`) see the document data. The band never paginates: content that overflows `header_height` just spills. |
| `footer` | element[] | Drawn in the bottom band on every page (may use `#PAGE#`/`#PAGES#`). Supports `paragraph`, `hr`, `image` (all three advance the band cursor), `move`, and `rect`/`line`/`ellipse` (drawn at the cursor — position them with `move`, which also grows the reserved band height). |
| `content` | element[] | The page body, laid out top to bottom. |

### Document options

```json
"options": {
  "header_height": 0,
  "watermark": { "text": "PAID", "angle": 45, "color": "e8c4c4", "fontSize": 96, "fontWeight": "bold" }
}
```

| key | type | default | meaning |
|---|---|---|---|
| `header_height` | number (pt) | `0` | Height of the reserved header band. |
| `margins` | number \| object | `56.693` (20 mm) | Page margins (pt). |
| `page` | string \| object | `a4` | Page size: a preset (`a4`/`letter`/`legal`/`a5`/`a3`) or `{ width, height }` in pt. |
| `orientation` | string | `portrait` | `landscape` swaps width/height. |
| `background` | string | — | Page background fill (hex, no `#`) painted behind every page, beneath any watermark and content. Omit for white paper. |
| `backgroundImage` | object | — | A full-page background image `{ "src": …, "pages"?: "all"/"first"/"last"/[…], "opacity"?: 0–1 }` — a cover photo or branded page. Drawn stretched to the page, above the `background` fill and below the watermark/content. `src` is any `image` source (URL/`file://`/`data:`). `opacity` (default `1`) fades it into a soft wash — set e.g. `0.15` for a faint stationery tint behind the content (clamped to `0`–`1`). |
| `watermark` | object | — | A diagonal stamp (e.g. `PAID`, `DRAFT`). Centered behind the content of every page by default; position, layering and page-scope are configurable. |
| `tagged` | bool | `false` | Emit a **tagged (accessible)** PDF — see [Accessible / tagged output](#accessible--tagged-output). Composes with `pdfa` **and** Factur-X: a tagged archival/e-invoice document declares PDF/UA-1 too. |
| `lang` | string | `en-US` | BCP-47 document language (e.g. `fr-FR`) written to the catalog. Used with `tagged`. |

**`page`** is a preset name (`a4`, `letter`, `legal`, `a5`, `a3`) or a custom
`{ "width": …, "height": … }` in points; **`orientation`: "landscape"** swaps the
two. **`margins`** is either a single number (all four sides) or an object overriding
individual sides; unset sides keep the 20 mm default. The **top** margin is the
gap above the header, the **bottom** margin the gap below the footer; `header_height`
reserves the header band below the top margin and the footer band auto-sizes above
the bottom margin. Lengths are points (1 mm ≈ 2.835 pt; A4 = 595×842 pt).

```json
"options": { "margins": { "top": 90, "left": 70, "right": 70, "bottom": 80 } }
"options": { "margins": 40 }
```

**`watermark`** fields:

| field | default | meaning |
|---|---|---|
| `text` | — | The stamp text (required). |
| `angle` | `45` | Rotation in degrees. |
| `color` | light grey | Hex fill, no `#`. |
| `fontSize` | `96` | Point size. |
| `fontWeight` | `bold` | `normal` / `bold`. |
| `front` | `false` | Draw **on top** of the content (an overlay) instead of behind it — use this so panels/images can't hide it. |
| `x` / `y` | page center | Explicit center point (pt). |
| `anchor` | `center` | Vertical placement when `y` is unset: `top` / `center` / `bottom`. |
| `pages` | `all` | Which pages to stamp: `"all"` / `"first"` / `"last"`, or a list of 1-based page numbers (`[1, 3]`). |

```json
"options": { "watermark": { "text": "PAID", "front": true, "anchor": "top", "pages": "first" } }
```

**Per-table watermark.** A `table` element can carry its own `watermark` (same
fields) — it's stamped centered over *that table's* box, always on top, so you can
mark a single table `PAID`/`VOID` without touching the rest of the page (`front`
and `pages` are ignored here — the stamp follows the table):

```json
{ "type": "table", "data": "items", "rows": [ ... ],
  "watermark": { "text": "PAID", "fontSize": 104, "color": "e8c4c4" } }
```

### Accessible / tagged output

Set `"tagged": true` (with an optional `"lang"`) to emit a **tagged PDF**: the
renderer wraps each piece of content in marked content with a **semantic role**
and adds the document structure assistive tech relies on — a `StructTreeRoot`,
`MarkInfo`, a `ParentTree`, per-page `StructParents`, logical tab order
(`/Tabs /S`), the document `/Lang`, and an XMP `pdfuaid` identifier.

```json
"options": { "tagged": true, "lang": "fr-FR" }
```

Roles are derived from the template:

| Content | Structure role |
|---|---|
| A bookmarked paragraph (`bookmark` / `bookmarkLevel`) | `H1`…`H6` (level = `bookmarkLevel`) |
| Any other paragraph | `P` |
| An `image`, `qr` or `barcode` | `Figure` (with `/Alt` from the image's `alt`) |
| Rules, watermarks, background art, running header/footer | `Artifact` (screen readers skip them) |

Give every meaningful `image` an `alt` — a `Figure` without alt text fails
PDF/UA, so a tagged render **warns** for each image missing one:

```json
{ "type": "image", "value": "logo.png", "width": 120, "alt": "Acme logo" }
```

Headings, paragraphs, figures, **lists** (`L › LI › LBody`) and **tables**
(`Table › TR › TD/TH`, with a `/Scope` on header cells) are all mapped to real
structure. A tagged document rendered with `pdfa` (or via `pdf_facturx*`)
validates as **both PDF/A-3b and PDF/UA-1** — checked in CI with the
[veraPDF](https://verapdf.org) reference validator.

**Tagging composes with PDF/A and Factur-X.** Set `options.tagged` together with
the `pdfa` option (or use `pdf_facturx*`) and the output carries **both** the
PDF/A-3b (`pdfaid`) and PDF/UA-1 (`pdfuaid`) identifiers over a single structure
tree — one file that is accessible, archival and (for Factur-X) machine-readable
at once. The tagging pass runs first; the PDF/A pass then merges the PDF/UA
identifier into the conformance metadata.

### Elements

Each element has a `type`. Lengths are in points (A4 = 595×842 pt).

**paragraph** — wrapped, aligned text; advances the cursor down. `options` accepts `alignment` (`left`/`right`/`center`/`justify`), `fontSize`, `fontWeight`, `italic`, `mono`, `color` (hex, no `#` — applies to the whole paragraph), `underline`/`strike` (bool — drawn in the text color), `lineHeight` (multiplier, engine default 1.2), `spacing` (pt gap added below the block — replaces trailing `move` elements), `minSpaceBelow` (keep-together: the paragraph only starts on this page if that many points remain below it — put it on headings so they're never orphaned at a page bottom), `link`/`linkTo`, `bookmark` + `bookmarkLevel`, and `anchor`. For per-run styling (mixed colors/weights on one line) use `spans` instead of `value`.

`justify` distributes the leftover width across word gaps; the paragraph's **last line stays left-aligned**. It works on both plain-`value` and `spans` paragraphs.

```json
{ "type": "paragraph", "value": "Invoice ${invoice.number}",
  "options": { "alignment": "left", "fontSize": 24, "fontWeight": "bold", "color": "0f766e" } }
```

**move** — relative cursor move. Positive `y` = down, negative `y` = up; positive `x` = right.

```json
{ "type": "move", "x": 0, "y": 24 }
```

**columns** — a multi-column flow block. Children fill column 1 to the bottom of the content region, then column 2, and so on (**sequential fill**); full-width flow resumes below. `count` (1–6, default 2) and `gap` (pt, default 12). A `page_break` inside is a **column break**; overflowing the last column starts a new page and restarts the set (the running header repeats). Paragraphs, lists, images, **tables and charts** all flow inside — a table that overflows a column continues in the next one with its header repeated. Nested `columns` are flattened.

```json
{ "type": "columns", "count": 2, "gap": 22, "content": [
  { "type": "paragraph", "value": "flows down column 1, then into column 2…" },
  { "type": "list", "items": ["lists flow too"] }
] }
```

**page_break** — force a new page at this point in the content flow (finishes the current page — footer included — and starts the next one with its header band). Replaces the old `{ "type": "move", "y": 3000 }` overflow trick. A trailing `page_break` with nothing after it leaves a final blank page.

```json
{ "type": "page_break" }
```

**image** — draw at the cursor. Sizing: `width` only → height derives from the aspect ratio; `height` only → width derives; **both** → the image scales to fit inside the `width`×`height` box ("contain", aspect preserved, never stretched). `value` is an `http(s)` URL, `file://` path, or `data:` URI. `alt` supplies the figure's alt text for [tagged output](#accessible--tagged-output). The cursor is not advanced. Raster formats (PNG, JPEG, WebP, GIF) **and SVG** are accepted — SVG is auto-detected and rasterised, so a vector logo or icon stays crisp at any placed size (`<text>` in the SVG uses the fonts from `font_dirs`). `http(s)` SVGs obey `fetch_images` like any other image. In an inline SVG `data:` URI, write colors as either a literal `#` (`fill='#0f766e'`) or the URL-encoded `%23` (`fill='%230f766e'`) — both work; SVG percentages like `width='50%'` are left intact.

```json
{ "type": "image", "value": "https://acme.example/logo.png", "width": 100 }
{ "type": "image", "value": "file://brand/logo.svg", "width": 120, "alt": "Acme logo" }
```

**table** — a grid of cells. Optional `data` binds the single template row to an array, repeating it per item. A non-empty `header_columns` repeats on every page the table spans; a non-empty **`footer_columns`** row closes the table AND repeats just above every intra-table page break (the "carried forward" subtotal band — its cells interpolate against the root data, not row items). `options.stripe` (hex) zebra-stripes every second body row; a cell's own `fill` paints its background (over the stripe — totals, highlights); `colspan` merges a cell across column slots (summary rows); `valign` (`top`/`middle`/`bottom`) positions a cell's content vertically in its row.

```json
{ "type": "table", "data": "items",
  "header_columns": [ { "text": "DESCRIPTION", "width": 280, "fontWeight": "bold",
                        "borderSides": { "bottom": "true" } } ],
  "rows": [ [ { "text": "${name}", "width": 280 },
              { "text": "${amount}", "width": 80, "alignment": "right" } ] ],
  "options": { "header": { "fillColor": "0F766E", "textColor": "FFFFFF" },
               "stripe": "f1f5f9", "padding_x": 6, "padding_y": 7 } }
```

A colspan totals row (3 columns, the label spans the first 2):

```json
{ "type": "table",
  "header_columns": [ { "text": "A", "width": 200 }, { "text": "B", "width": 100 }, { "text": "C", "width": 100 } ],
  "rows": [ [ { "text": "TOTAL DUE", "colspan": 2, "alignment": "right", "fontWeight": "bold" },
              { "text": "2,140.00 EUR", "alignment": "right", "fill": "fef3c7" } ] ] }
```

**hr** — a horizontal rule across the content width (or `width` pt). Advances the cursor.

```json
{ "type": "hr", "color": "cccccc", "thickness": 0.5, "width": 515 }
```

**rect** — a filled and/or stroked rectangle at the cursor (top-left). Useful for header bands and boxes. Does **not** advance the cursor — position it with `move`.

```json
{ "type": "rect", "width": 515, "height": 26, "fill": "f4f4f5", "border": "000000", "borderWidth": 0.5 }
```

**line** — a stroked segment from the cursor to `cursor + (dx, dy)`. Does not advance the cursor.

```json
{ "type": "line", "dx": 200, "dy": 0, "color": "cccccc", "width": 0.5 }
```

**ellipse** — a filled and/or stroked ellipse (circle when `rx == ry`) whose bounding-box top-left is the cursor. Does not advance the cursor.

```json
{ "type": "ellipse", "rx": 6, "ry": 6, "fill": "16a34a" }
```

`hr`, `line`, and `rect`/`ellipse` borders accept a **`dash`** array (pt on/off lengths, e.g. `[3, 2]`) for dashed/dotted strokes; `rect` accepts a **`radius`** (pt) for rounded corners.

**qr** — a QR code (square, side = `width` pt) rendered at the cursor. Does not advance the cursor. Either a SEPA "scan-to-pay" GiroCode or arbitrary text — see [Payment QR](#payment-qr-scan-to-pay).

```json
{ "type": "qr", "kind": "epc", "iban": "${payment.iban}", "name": "${payment.name}",
  "amount": "${payment.amount}", "currency": "${payment.currency}",
  "remittance": "${invoice.number}", "width": 110 }
```

**barcode** — a 1D barcode rasterised at the cursor, sized `width` × `height` pt. Does not advance the cursor. `value` is `${…}`-interpolated. `symbology` is one of `code128` (any printable ASCII), `ean13` (12 digits — the check digit is computed), `ean8` (7 digits), or `code39` (uppercase letters, digits, and `- . $ / + % space`). Set `humanReadable: true` to print the value as a caption below the bars. Invalid data (wrong length, unsupported characters) is **skipped with a warning**, not fatal.

```json
{ "type": "barcode", "symbology": "code128", "value": "ORDER-${order.id}",
  "width": 220, "height": 56, "humanReadable": true }
```

**list** — a bulleted or numbered list; flows like a paragraph and advances the cursor. `ordered` (default `false`) numbers the items (`1.`, `2.`, …) from `start` (default `1`); otherwise each item gets a bullet (`marker`, default `"•"`). Items are plain strings, or objects carrying a `text`/`spans` body and/or a nested `list` (rendered indented one level deeper). `indent` (pt) and `spacing` (pt between items) tune the layout, and `options` styles the item text (the same `fontSize`/`fontWeight`/`alignment` as a paragraph).

```json
{ "type": "list", "ordered": true, "options": { "fontSize": 11 }, "items": [
  "Grind the beans",
  "Boil water to 94°C",
  { "text": "Brew", "list": { "items": ["Bloom 30s", "Pour to 250 g"] } },
  { "spans": [ { "text": "Enjoy " }, { "text": "responsibly", "italic": true } ] }
] }
```

**chart** — a bar, line, pie, or donut chart drawn from the data. Occupies `width` × `height` pt at the cursor (plus an optional `title` above) and advances the cursor below it. `kind` is `bar`, `line`, `pie`, or `donut` (a pie with a ring cutout — whatever is behind the chart shows through the hole). Points come either from a **data binding** — `data` names an array in the data document and `label`/`value` name the fields read from each item — or from inline `points`. `colors` (hex, no `#`) are cycled across points; a built-in palette is used when omitted. For `pie`/`donut`, `legend` adds a swatch + percentage list; for `bar`/`line`, `axis` draws axis lines and category labels, and `gridlines: true` adds horizontal value-axis gridlines with tick labels.

**Multiple series.** Instead of a single `value`, give `values` — an array of `{ field, name?, color? }`. `data`/`label` still name the array and the category field, and each series reads its own `field` from every item. Bars render **grouped** side-by-side (or **stacked** with `mode: "stacked"`); `line` draws one line per series; both show a legend of the series `name`s.

```json
{ "type": "chart", "kind": "bar", "data": "quarters", "label": "q", "gridlines": true,
  "values": [ { "field": "fy24", "name": "FY 2024", "color": "94a3b8" },
              { "field": "fy25", "name": "FY 2025", "color": "0f766e" } ],
  "width": 460, "height": 160 }
```

```json
{ "type": "chart", "kind": "bar", "title": "Revenue by month",
  "data": "months", "label": "name", "value": "revenue", "width": 360, "height": 160 }

{ "type": "chart", "kind": "pie", "width": 360, "height": 160, "points": [
  { "label": "Rent", "value": 1200 }, { "label": "Payroll", "value": 3400 },
  { "label": "Cloud", "value": 800 } ] }
```

### Control flow: repeat, if / unless

Two structural elements make the whole document data-driven, not just table rows:

**repeat** — lay out `content` (an array of elements) once per item of the `data` array, with `${field}` scoped to each item (falling back to the root) — the block-level analogue of a data-bound table row. A missing or empty array renders nothing.

```json
{ "type": "repeat", "data": "invoices", "content": [
  { "type": "paragraph", "spans": [ { "text": "${number}", "fontWeight": "bold" }, { "text": " — ${customer}" } ] },
  { "type": "hr" }
] }
```

**if** / **unless** — render `content` only when a condition holds (`if`) or fails (`unless`); an optional `else` array is the other branch. The condition reads `${when}`: with `equals` it's a string-equality test, otherwise a truthiness test (a value is falsy when it is missing, empty, `false`, `0`, or `null`).

```json
{ "type": "if", "when": "paid", "equals": "true",
  "content": [ { "type": "paragraph", "value": "PAID IN FULL" } ],
  "else":    [ { "type": "paragraph", "value": "Balance due" } ] }

{ "type": "unless", "when": "items",
  "content": [ { "type": "paragraph", "value": "No line items." } ] }
```

> A `table` or `chart` nested inside a `repeat` still binds its own `data` key against the **top-level** data document, not the current item.

### Cells: text & rich

A cell is a simple **text** cell, or a **rich** cell whose `content` stacks multiple items — text lines *and* images — in one cell.

```json
{ "content": [
    { "type": "text",  "value": "${company.name}", "fontSize": 12, "fontWeight": "bold" },
    { "type": "text",  "value": "${company.city}", "fontSize": 10 },
    { "type": "image", "value": "file://logo.png", "width": 80 }
  ],
  "width": 200, "alignment": "left",
  "borderSides": { "right": "false", "left": "false", "top": "false", "bottom": "false" } }
```

### Styling & colours

- `alignment` — `left` / `right` / `center` / `justify` (case-insensitive; `justify` works on plain-`value` and `spans` paragraphs, and leaves the last line left-aligned).
- `fontSize` (pt), `fontWeight` — `normal` / `bold`.
- `link` — an external URL (on a paragraph's `options` or a text cell's style). The text becomes a clickable, borderless link annotation; the Factur-X output stays PDF/A-3b conformant (the Print flag is set automatically). Example: `{ "type": "paragraph", "value": "Pay online", "options": { "link": "https://pay.example/42" } }`.
- `bookmark` / `anchor` / `linkTo` (paragraph `options`) — navigation. `bookmark` adds a PDF outline (sidebar) entry; **`bookmarkLevel`** nests it (1 = top; a level-2 entry nests under the last level-1, like headings); `anchor` names a jump target; `linkTo` makes the text a clickable internal jump to an `anchor`. Great for multi-page statements / a clickable table of contents. (`linkTo` also accepts `link_to`.)
- `width` — column width (pt). Width-less columns split the remainder; over-wide rows scale to fit.
- `borderSides` — `{ "top": "true", "bottom": "false", ... }`; strings or bools. When present, omitted sides default to `true`; when absent entirely, no borders.
- `borderColor` — hex without `#`, 3 or 6 digits (`"fff"`, `"EEEEEE"`). Default light grey.
- `fill` (cell) — background fill for one cell (hex). Painted over the zebra stripe and the header band, beneath borders and text.
- `valign` (cell) — vertical alignment within the row: `top` / `middle` (default, optically centered) / `bottom`.
- `colspan` (cell) — merge the cell across that many column slots; following cells shift right. Define column widths with `header_columns` (or a spanless row) and use `colspan` in body/summary rows.
- `options.stripe` (table) — zebra fill (hex) behind every second body row. Header rows are never striped.
- `header.fillColor` / `textColor` / `borderColor` — the header row's band fill, text colour, and border. Body text is always black; accents come from bands, stripes, and rules.

### Interpolation

- `${a.b.c}` — dotted path into the data; missing paths render empty (with a warning).
- `$${...}` — a **literal** `${...}` (double the `$`): the token is printed verbatim instead of interpolated. Use it to show template syntax in the output itself (a code sample, documentation) or wherever a `$` legitimately precedes a `{`. Works in both `value` and `spans` text.
- Inside a data-bound table, `${field}` resolves against the row item first, then the root.
- `#PAGE#` / `#PAGES#` (alias `#TOTAL_PAGE#`) — page tokens, substituted after pagination. They work in **footer, header, and body** paragraphs (`value` form; a `spans` paragraph renders them literally).
- `#PAGE_OF:anchor#` — the 1-based page number of the paragraph carrying `"anchor": "…"`. Combine with `linkTo` for a **table of contents with real page numbers**: `{ "value": "Charts ..... p. #PAGE_OF:sec-charts#", "options": { "linkTo": "sec-charts" } }`. An unknown anchor renders empty with a warning.
- Token lines defer to a second pass, so their link annotations cover the full line box and underline/strike are skipped (the substituted width isn't known yet).

### Inline rich text

A `paragraph` may carry `spans` instead of `value` to mix weight, size, color, and inline links **within one wrapped flow**. Each span inherits the paragraph `options` for fields it omits; lines wrap together and the line height follows the largest span on each line.

```json
{ "type": "paragraph", "options": { "fontSize": 12 }, "spans": [
  { "text": "Amount due: " },
  { "text": "EUR 600.00", "fontWeight": "bold", "color": "0F766E", "fontSize": 16 },
  { "text": "  —  " },
  { "text": "pay now", "link": "https://pay.example/42", "color": "2563eb" },
  { "text": " (", "italic": true }, { "text": "see README", "mono": true }, { "text": ")", "italic": true }
] }
```

Span fields: `text` (required), `fontSize`, `fontWeight`, `italic` (bool), `mono` (bool — monospace, e.g. for `code`), `color` (hex), `link` (external URL), `underline` / `strike` (bool — the stroke follows the span's color, so a red `strike` span reads as a redlined price). `italic` and `mono` need the matching faces in the font dir (Titillium italics + JetBrains Mono ship by default); if a face is missing the span degrades to the nearest available one.

**From markdown.** [`Markdown.to_spans(md)`](markdown) parses inline markdown — `**bold**`, `*italic*`, `` `code` `` (→ mono), `[text](url)` — into exactly this spans array, so you can author rich text as markdown:

```soli
let spans = Markdown.to_spans("Pay **now**, read the `README`, see [docs](https://x).")
let template = { "fonts": ["titillium"],
  "content": [ { "type": "paragraph", "spans": spans } ] }
let pdf = pdf_render(template.to_json(), "{}")
```

### Payment QR (scan-to-pay)

**How it works.** An EPC QR encodes a **SEPA bank transfer** — not a card charge or a payment link. The customer opens their **banking app**, scans the code, and the transfer is **pre-filled** (payee name, IBAN, BIC, amount, reference); they just confirm. It's a *push* payment the payer approves in their own bank, so nothing is charged automatically and there's no real-time callback — you reconcile incoming transfers by the **remittance reference** (your invoice/receipt number). It's **EUR / SEPA only**, recognized by SEPA-area banking apps (ubiquitous as the "GiroCode" in the DACH region); apps outside SEPA generally won't read it. The code is built and rasterized into the PDF locally — no network call.

Under the hood it's a fixed EPC069-12 text block (not a URL): a `BCD` service tag + version, `SCT` (SEPA Credit Transfer), then BIC, beneficiary name, IBAN, `EUR<amount>` (leave the amount empty to let the payer enter it), and your reference. The whole payload must stay ≤ 331 bytes and uses error-correction level **M**, as EPC mandates.

A `qr` element renders a QR code as a raster image (square, side = `width` pt). Two kinds:

- **`"kind": "epc"`** (default) — an **EPC069-12 "GiroCode"** SEPA Credit Transfer. The buyer scans it in their banking app and the payee/IBAN/amount/reference are pre-filled. All string fields are `${…}`-interpolated.

  | field | notes |
  |---|---|
  | `name` | Beneficiary (seller) name. Required. ≤ 70 chars. |
  | `iban` | Beneficiary IBAN. Required. ≤ 34 chars. |
  | `bic` | Optional within the EEA. |
  | `amount` | Decimal; re-formatted to 2 dp. Must be **EUR** and within `0.01…999999999.99`. |
  | `currency` | Must be `EUR` (EPC is EUR-only); defaults to `EUR`. |
  | `remittance` | Unstructured reference (e.g. the invoice number). ≤ 140 chars. |
  | `purpose` | Optional 4-char purpose code. |

  Invalid input (non-EUR, missing IBAN, over-long fields) is **skipped with a warning**, not fatal.

- **`"kind": "text"`** — encodes `value` verbatim.

When you use `pdf_facturx_from_invoice`, a ready-made `payment` block is exposed in the render data so the QR binds straight to the invoice — give the seller an `iban` (and optional `bic`):

```json
{ "type": "qr", "kind": "epc",
  "name": "${payment.name}", "iban": "${payment.iban}", "bic": "${payment.bic}",
  "amount": "${payment.amount}", "currency": "${payment.currency}",
  "remittance": "${payment.remittance}", "width": 110 }
```

`payment.amount` is the amount due with **no currency symbol** (e.g. `600.00`), `payment.currency` the ISO code, and `payment.remittance` the invoice number.

### Fonts

No fonts are embedded in the binary. Put font files in a `font/` folder (default `font_dirs`). The template's `fonts` field names the primary family; its Regular/Bold are resolved from the files, and any other loaded font becomes a fallback (e.g. a CJK font). Uncovered characters are dropped with a warning.

```
font/
  TitilliumWeb-Regular.ttf
  TitilliumWeb-Bold.ttf
  NotoSansJP-Regular.ttf   # optional CJK fallback
```

---

## Factur-X & PDF/A-3b

The `profile` sets the embedded file's `/AFRelationship` and XMP `fx:ConformanceLevel`; match it to your XML's BT-24 guideline id.

| profile | Contents | AFRelationship |
|---|---|---|
| `minimum` | Parties + totals | Data |
| `basicwl` | Header + totals, no lines | Data |
| `basic` | Line items (subset) | Alternative |
| `en16931` | Full EN 16931 (default) | Alternative |
| `extended` | EN 16931 + extras | Alternative |

Validate:

```bash
verapdf -f 3b invoice.pdf            # PDF/A-3b → PASS (146/146 rules)
java -jar Mustang-CLI.jar --action validate --source invoice.pdf
```

### Typed invoice document

`pdf_facturx_from_invoice` takes a typed invoice instead of separate data + XML. Totals and the VAT breakdown are computed from the lines, then mapped onto the template (`invoice.*`, `company.*` = seller, `customer.*` = buyer, `items[]`, `total.*`, `infos.text`) and emitted as CII XML.

```json
{
  "number": "#12345",
  "issue_date": "2025-11-28",
  "due_date": "2025-12-28",
  "currency": "EUR",
  "note": "Thank you for your business.",
  "seller": {
    "name": "PDFx", "address_line": "1 Rue des Champs-Élysées",
    "postcode": "75000", "city": "PARIS",
    "country": "FR", "country_name": "France",
    "phone": "+33 6 12 34 80 32", "vat_id": "FRXX999999999"
  },
  "buyer": {
    "name": "John Doe", "address_line": "123 Main St",
    "postcode": "12345", "city": "NYC", "country": "US", "country_name": "USA"
  },
  "lines": [
    { "name": "Item 1", "quantity": 1, "unit_price": 100, "vat_rate": 20.0 },
    { "name": "Item 2", "quantity": 2, "unit_price": 200, "vat_rate": 20.0 }
  ],
  "allowances": [
    { "reason": "Volume discount", "percent": 10, "vat_rate": 20.0 }
  ],
  "charges": [
    { "reason": "Shipping", "amount": "20.00", "vat_rate": 20.0 }
  ],
  "payment_terms": "30 days net"
}
```

| Field | Notes |
|---|---|
| `number`, `issue_date`, `currency` | Required. `issue_date` is `YYYY-MM-DD`. |
| `due_date`, `note`, `type_code` | Optional. `type_code` defaults to `380` (commercial invoice); accepted codes: `380`, `381` (credit note), `384`, `389`, `261`, `386`. |
| `currency_symbol` | Optional; otherwise derived from the code (`EUR`→`€`, `USD`→`$`, …). |
| `prepaid` | Optional amount already paid; subtracted to give the amount due and emitted as `TotalPrepaidAmount` (BT-113). |
| `allowances[]` | Optional document-level discounts (BG-20): `reason` + **exactly one** of `amount` / `percent` (of the line-net total), plus `vat_rate`/`vat_category` (default `S`). Reduce the tax basis of their VAT group. |
| `charges[]` | Optional document-level charges — shipping, fees (BG-21). Same shape as `allowances`; they increase the tax basis. A charge/allowance with a rate no line uses gets its own VAT-breakdown row. |
| `payment_terms` | Optional free-text payment terms (BT-20), e.g. `"30 days net"`. Emitted with `due_date` in the CII `SpecifiedTradePaymentTerms` block. |
| `seller` / `buyer` | `name`, `address_line`, `postcode`, `city`, `country` (ISO-2), `country_name`, `phone`, `vat_id`. The seller's `vat_id` is required by EN 16931 when VAT is charged. |
| `seller.iban` / `seller.bic` | Optional. When present, exposed as the `payment.*` render-data block for an EPC [scan-to-pay QR](#payment-qr-scan-to-pay). |
| `lines[]` | `name`, `unit_price`, `quantity` (default `1`), `vat_rate` (percent), `unit_code` (default `C62`), `vat_category` (default `S`). |

Amounts accept a number (`100`, `100.5`) or a numeric string (`"100.50"`) and are kept exact to the cent.

**Credit notes.** Set `"type_code": "381"` (or `261` for self-billed): amounts stay **positive** — in CII the type code carries the semantics, there is no separate credit-note document. The render data exposes `invoice.type_code` and a ready-made `invoice.type_label` (`"Invoice"` / `"Credit note"`) for the template's title line.

**Render-data paths.** Besides `items[]`, the computed figures land on `total.*`: `total.amount` (line total), `total.discount` (BT-107), `total.charges` (BT-108), `total.taxable` (the tax basis, BT-109 — differs from `total.amount` once allowances/charges exist), `total.vat`, `total.due_amount`. Each allowance/charge is also exposed for display in `discounts[]` / `charges[]` as `{ reason, amount, percent }` — bind them with a `repeat` or a data-bound `table` for the totals card:

```json
{ "type": "repeat", "data": "discounts", "content": [
  { "type": "paragraph", "value": "${reason}  −${amount}", "options": { "alignment": "right" } }
] }
```

---

## Performance

Generation is in-process and CPU-bound. Benchmark a controller that renders an invoice on every request with [oha](https://github.com/hatoo/oha):

```soli
def pdf_bench
  let pdf = pdf_render(slurp("pdf/template.json"), slurp("pdf/data.json"),
                       { "fetch_images": false })
  return { "status": 200, "headers": {"Content-Type": "text/plain"},
           "body": "generated " + len(pdf) + " base64 bytes" }
end
```

```bash
oha -n 3000 -c 50 http://127.0.0.1:8080/pdf-bench
```

A Latin-only invoice renders in a couple of milliseconds (~135 KB). Framework overhead is negligible (`/health` sustains tens of thousands of req/s) — the cost is the PDF. Embedding a CJK fallback font is larger/slower because printpdf embeds whole fonts.
