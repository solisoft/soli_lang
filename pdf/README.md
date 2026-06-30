# soli-pdf

Render a PDF from a **JSON layout template + JSON data**, then embed a
caller-provided **Factur-X (EN 16931) CII XML** to produce a valid
**PDF/A-3b electronic invoice**.

The output opens as a normal PDF *and* validates as Factur-X / ZUGFeRD 2.x — the
generated sample passes **veraPDF PDF/A-3b** with 0 failures (146/146 rules).

## Pipeline

```
template.json ─┐
data.json    ─┤→ [render engine] → PDF bytes ─┐
              │   parse · interpolate · layout │→ [facturx] → PDF/A-3b invoice
              │   fonts/fallback · paginate    │   + factur-x.xml · XMP · sRGB OutputIntent
facturx.xml ──────────────────────────────────┘   + /AF · CIDToGIDMap fix · PDF 1.7
```

1. **Render engine** — turns the template + data into a normal PDF
   (`printpdf` backend, wrapped behind one module).
2. **Factur-X step** — post-processes the PDF with `lopdf` to add everything
   PDF/A-3b + Factur-X requires and embeds the CII XML.

## Usage

### Library

```rust
use soli_pdf::{generate_facturx, FacturxMetadata, Profile, RenderOptions};

let pdf = generate_facturx(
    &template_json,        // &[u8]
    &data_json,            // &[u8]
    &facturx_xml,          // &[u8] — caller-provided CII XML
    Profile::En16931,
    &FacturxMetadata { title: "Invoice #12345".into(), ..Default::default() },
    &RenderOptions::default(),
)?;
std::fs::write("invoice.pdf", pdf)?;
```

Other entry points:

- `render_to_bytes(template, data, opts)` — just the visual PDF.
- `render_with_warnings(...)` — PDF + non-fatal `RenderWarning`s (missing
  placeholders, skipped images, missing glyphs).
- `facturx::embed_facturx(pdf, xml, profile, meta)` — embed into existing PDF
  bytes.
- `generate_facturx_from_invoice(template, &invoice, profile, meta, opts)` —
  **single source of truth** (see below): a typed `Invoice` drives both the
  visual PDF and a computed, consistent CII XML.

### Single source of truth (`Invoice`)

Instead of hand-authoring the CII XML and hoping it matches the rendered
document, build one typed [`Invoice`] and let the library produce **both** the
visual PDF *and* the embedded EN 16931 XML from it. Totals and the VAT breakdown
are computed from the line items, so the human-readable and machine-readable
representations can never disagree:

```rust
use soli_pdf::{generate_facturx_from_invoice, Invoice, Profile, FacturxMetadata, RenderOptions};

let invoice = Invoice::parse(&invoice_json)?;     // or build the struct directly
let pdf = generate_facturx_from_invoice(
    &template_json, &invoice, Profile::En16931,
    &FacturxMetadata { title: "Invoice #12345".into(), ..Default::default() },
    &RenderOptions::default(),
)?;
```

- `invoice.to_render_data()` → the JSON the template interpolates (`invoice.*`,
  `company.*`, `customer.*`, `items[]`, `total.*`, `infos.*`).
- `invoice.to_cii_xml(profile)` → EN 16931 CII XML with line items, a VAT
  breakdown grouped by (category, rate), and computed totals
  (`LineTotal`/`TaxBasis`/`TaxTotal`/`GrandTotal`/`DuePayable`).

Monetary inputs use an exact `Amount` (hundredths) that deserialises from a JSON
number or numeric string and formats to two decimals. The generated XML mirrors
the structure of the validated `tests/fixtures/factur-x.xml` reference and the
result still passes **veraPDF PDF/A-3b (146/146)**. A sample invoice is at
`tests/fixtures/invoice.json`.

### CLI

```bash
cargo run --bin render_pdf -- \
  --template tests/fixtures/template.json \
  --data     tests/fixtures/data.json \
  --xml      tests/fixtures/factur-x.xml \
  --profile  en16931 \
  --out      invoice.pdf
# --no-images to skip remote image fetches (offline/deterministic)

# Single source of truth: one invoice JSON → PDF + computed, consistent CII XML
# (no separate --data/--xml; the XML is generated, not supplied).
cargo run --bin render_pdf -- \
  --template tests/fixtures/template.json \
  --invoice  tests/fixtures/invoice.json \
  --profile  en16931 \
  --font-dir fonts \
  --out      invoice.pdf
```

## Template format

A template is a JSON object with five top-level keys (all optional):

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
| `fonts` | `string[]` | Font families to load; the first is the primary text face, the rest are fallbacks. |
| `options` | object | Document options — see [Document options](#document-options). |
| `header` | `element[]` | Drawn in the reserved top band on every page (its own cursor). |
| `footer` | `element[]` | Drawn in the bottom band on every page; may use `#PAGE#`/`#TOTAL_PAGE#`. |
| `content` | `element[]` | The page body, laid out top-to-bottom. |

### Document options

| key | type | default | meaning |
|---|---|---|---|
| `header_height` | number (pt) | `0` | Height of the reserved header band. |
| `margins` | number \| object | `56.693` (20 mm) | Page margins (pt). A single number sets all four sides; an object `{ top, right, bottom, left }` overrides individual sides (unset sides keep the default). |
| `page` | string \| object | `a4` | Page size: a preset (`a4`/`letter`/`legal`/`a5`/`a3`) or `{ width, height }` in pt. |
| `orientation` | string | `portrait` | `landscape` swaps width/height. |
| `watermark` | object | — | A diagonal stamp drawn behind every page (see below). |

The **top** margin is the gap above the header; the **bottom** margin the gap below
the footer. `header_height` then reserves the header band below the top margin, and
the footer band auto-sizes above the bottom margin. Lengths are points
(1 mm ≈ 2.835 pt); A4 is 595×842 pt.

```json
"options": { "margins": { "top": 90, "left": 70, "right": 70, "bottom": 80 } }
"options": { "margins": 40 }
```

`watermark` fields: `text` (required), `angle` (deg, default `45`), `color`
(hex, default light grey), `fontSize` (pt, default `96`), `fontWeight`
(`normal`/`bold`, default `bold`). It is centered on the page and rendered
beneath the content.

### Elements

Every element is `{ "type": <name>, … }`. Lengths are points (A4 = 595×842 pt).

| type | fields |
|---|---|
| `paragraph` | `value` (`${path}` interpolated) **or** `spans[]` (inline rich text, below), `options { alignment, fontSize, fontWeight, link, bookmark, anchor, linkTo }` |
| `move` | `x`, `y` — relative cursor delta (positive `y` = down) |
| `image` | `value` (http(s) / `file://` / `data:` URI), `width` (pt; height auto) |
| `table` | `data?` (binding key), `header_columns[]`, `rows[][]`, `options { header, padding_x, padding_y }` |
| `hr` | `color?`, `thickness?` (pt, default `0.5`), `width?` (pt), `dash?` (pt on/off, e.g. `[3,2]`) — a rule; **advances** the cursor |
| `rect` | `width`, `height`, `fill?`, `border?`, `borderWidth?` (default `0.5`), `radius?` (rounded corners), `dash?` — placed at the cursor; no advance |
| `line` | `dx`, `dy`, `color?`, `width?` (default `0.5`), `dash?` — a segment from the cursor to `cursor+(dx,dy)`; no advance |
| `ellipse` | `rx`, `ry`, `fill?`, `border?`, `borderWidth?`, `dash?` — at the cursor (bbox top-left); a circle has `rx == ry`; no advance |
| `qr` | a QR code — see [Payment QR](#payment-qr-scan-to-pay); no advance |

`paragraph`, `image`, `hr`, `rect`, `line`, `ellipse`, `qr` placement uses the
cursor; only `paragraph`, `hr`, and `table` advance it vertically (use `move` to
position the others).

### Inline rich text

A `paragraph` may use `spans` instead of `value` to mix weight, size, color, and
inline links *within* one wrapped flow. Each span inherits the paragraph
`options` for the fields it omits:

```json
{ "type": "paragraph", "options": { "fontSize": 12 }, "spans": [
  { "text": "Amount due: " },
  { "text": "EUR 600.00", "fontWeight": "bold", "color": "0F766E", "fontSize": 16 },
  { "text": "  —  " },
  { "text": "pay now", "link": "https://pay.example/42", "color": "2563eb" }
] }
```

Lines wrap together; line height and baseline follow the largest span on each line.

### Cells: text & rich

Table cells are either a simple **text** cell or a **rich** cell that stacks
multiple items (text lines *and* images) in one cell:

```json
{ "text": "...", "width": 120, "fontSize": 10, "alignment": "right",
  "fontWeight": "bold", "borderSides": {…}, "borderColor": "EEEEEE", "link": "https://…" }

{ "content": [ { "type": "text", "value": "…", "fontSize": 12, "fontWeight": "bold" },
               { "type": "image", "value": "file://logo.png", "width": 80 } ],
  "width": 200, "alignment": "left", "borderSides": {…} }
```

### Styling, interpolation & colors

- **Text styling**: `alignment` (`left`/`right`/`center`, case-insensitive),
  `fontSize` (pt), `fontWeight` (`normal`/`bold`).
- **`link`**: an external URL on a `paragraph`'s `options` or a text cell's style;
  the text becomes a clickable link annotation (borderless). PDF/A-3b conformant.
- **Navigation** (paragraph `options`): `bookmark` adds a PDF outline entry,
  `anchor` names a jump target, and `linkTo` makes the text a clickable internal
  jump to an `anchor`. All stay PDF/A-3b conformant.
- **`width`**: column width (pt); width-less columns split the remainder, over-wide
  rows scale to fit.
- **`borderSides`**: `{ top, bottom, left, right }`, values strings
  (`"true"`/`"false"`) or bools. When the object is present, omitted sides default
  to `true`; absent entirely ⇒ no borders.
- **Colors** (`color`/`fill`/`border`/`borderColor`/`fillColor`/`textColor`) are
  hex without `#`, 3- or 6-digit (`"fff"`, `"EEEEEE"`).
- **Interpolation**: `${a.b.c}` resolves against the data document's `data`
  object. Inside a `data`-bound table, `${field}` resolves against each row item
  first, then the root. Missing paths render empty (with a warning).
- **Footer** paragraphs may contain `#PAGE#` / `#TOTAL_PAGE#`, substituted in a
  second pass once the page count is known (alignment is recomputed).

### Payment QR (scan-to-pay)

A `qr` element renders a QR code as a raster image at the cursor (square, side =
`width` pt). Two kinds:

- `"kind": "epc"` (default) — an **EPC069-12 "GiroCode"** SEPA Credit Transfer the
  buyer scans in their banking app. Fields (all `${…}`-interpolated):
  `name`, `iban`, `bic?`, `amount`, `currency` (must be `EUR`), `remittance`,
  `purpose?`. Invalid input (non-EUR, missing IBAN, …) is skipped with a warning
  rather than aborting the render.
- `"kind": "text"` — encodes `value` verbatim.

```json
{ "type": "qr", "kind": "epc",
  "name": "${payment.name}", "iban": "${payment.iban}",
  "amount": "${payment.amount}", "currency": "${payment.currency}",
  "remittance": "${invoice.number}", "width": 110 }
```

`generate_facturx_from_invoice` exposes a ready-made `payment` block in the render
data (`payment.{name,iban,bic,amount,currency,remittance}`, amount with no
currency symbol) so the QR binds straight to the invoice.

### Coordinate model

The engine uses a **top-left origin, y-increasing-downward** cursor (points).
`move` adds its delta directly, so **negative `move.y` goes up**, positive goes
down — e.g. the sample places the logo top-right with
`move {x:412,y:-30}` … image … `move {x:-412,y:-60}`. Conversion to PDF's
native bottom-left space happens in exactly one place (`geometry::Page::to_pdf_y`).
Templates author absolute-ish layouts via these relative moves; tune the offsets
to your margins (default A4, 20 mm margins).

## Factur-X / PDF-A-3b

- Default profile **EN 16931** (`Profile` also covers Minimum, BasicWl, Basic,
  Extended). The profile sets `/AFRelationship` (Alternative vs Data) and the
  XMP `fx:ConformanceLevel`; it should match the embedded XML's BT-24 guideline.
- The library **embeds caller-provided XML** — it does not synthesize the CII
  from the data JSON. A complete EN 16931 sample is at
  `tests/fixtures/factur-x.xml`.
- What the facturx step adds: PDF version 1.7, the `factur-x.xml` embedded-file
  stream, `/Filespec` + `/EmbeddedFiles` name tree + `/AF`, an sRGB
  `OutputIntent` (bundled ICC), the PDF/A + Factur-X XMP packet, an Info dict +
  trailer `/ID`, and an explicit `/CIDToGIDMap /Identity` on every embedded
  CIDFont (required by PDF/A; printpdf omits it).

### Validating output

```bash
# PDF/A-3b structure, fonts, OutputIntent, XMP:
verapdf -f 3b invoice.pdf            # this crate's sample → PASS (146/146)

# Factur-X XML + profile (XSD + Schematron):
java -jar Mustang-CLI.jar --action validate --source invoice.pdf
```

## Sample-data caveats (for the bundled `factur-x.xml`)

The example amounts are shown with `$`, so the sample uses **USD**; switch
`InvoiceCurrencyCode`/`currencyID` to `EUR` for an EU invoice. EN 16931 requires
a **seller VAT id (BT-31)** when VAT is charged — the sample uses a placeholder
`FRXX999999999`. The buyer address string is split heuristically. Replace these
with real values for production invoices.

## Fonts

**No fonts are bundled in the binary.** Faces are loaded at runtime from the
directories in `RenderOptions::font_dirs` (CLI `--font-dir`, default `./fonts`
then `./font`). The template's `fonts: ["titillium"]` selects which loaded
family is the primary text face; its Regular/Bold styles are resolved from the
files, and any other loaded font (e.g. a CJK font) becomes a **fallback** for
characters the primary can't cover. Characters no loaded font covers are dropped
with a warning (keeping the PDF valid).

This repo ships runtime fonts (not embedded) in `fonts/` (Titillium Web,
SIL OFL) and the `lang/font/` folder also carries **Noto Sans JP** for CJK
coverage. To render the sample's `Invoice こんにちは世界` title, point a font dir
at a folder containing a CJK font:

```bash
cargo run --bin render_pdf -- ... --font-dir ../lang/font
```

printpdf 0.9 disables subsetting upstream, so the crate **pre-subsets each face
itself** (retain-GID, via the `subsetter` crate) before handing the bytes to
printpdf — a Latin-only invoice is ~134 KB. A retain-GID subset keeps the
`loca`/`hmtx`/`cmap` tables full-size, so a CJK fallback (whose bulk is those
tables) only shrinks modestly and a CJK invoice is still several MB.

## Benchmarks

`cargo bench` (criterion). Indicative on this machine:

| benchmark | time | output |
|---|---|---|
| `parse_template` | ~44 µs | — |
| `parse_data` | ~4.4 µs | — |
| `font_registry_new` | ~0.8 µs | — (ttf-parser is zero-copy/lazy) |
| `render_to_bytes_latin` (Latin-only) | **~1.4 ms** | **~134 KB** |
| `render_to_bytes_cjk` (sample, with CJK) | ~145 ms | ~5.9 MB |
| `embed_facturx` | ~1.2 ms | — |

The Latin-only path — the common case — renders in ~1.4 ms to a ~134 KB PDF.
The CJK case is ~100× slower and larger: even after the crate's retain-GID
subsetting, a CJK fallback's `loca`/`hmtx`/`cmap` tables stay full-size, so most
of the ~5.7 MB face is still embedded.

## Tests

`cargo test` — model parsing of the real fixtures, interpolation/row-scope,
coordinate transform, color parsing, font fallback, and a full
render→embed→reparse pipeline that round-trips the embedded XML and checks the
PDF/A-3b structure.

## Limitations / future work

- CII generation covers the EN 16931 fields in the bundled reference (parties,
  lines, VAT breakdown, totals). Allowances/charges, payment means/terms, and
  document references aren't emitted yet — `embed_facturx` still accepts a
  hand-authored XML for those cases.
- CJK invoices stay large: printpdf 0.9 disables subsetting and the crate's
  retain-GID workaround can't shrink a CJK face's bulk (`loca`/`hmtx`/`cmap`).
- The sample template's title/company spacing reflects its own `move` offsets
  against the default margins; tune offsets per template.

## Third-party assets

See `THIRD_PARTY.md`. Bundled fonts are SIL OFL 1.1; the sRGB ICC profile is
CC0.
