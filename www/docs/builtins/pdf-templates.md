# Invoice & Quote Templates

Eight ready-to-copy billing documents rendered by the PDF engine. Each answers the same three questions — who, how much, by when — with a different structure, because a VAT-compliant invoice, a builder's quote and a metered subscription bill are not the same document wearing different colours.

The templates live in `www/public/pdf-samples/` as a `<name>.template.json` + `<name>.data.json` pair. Rendered page: [`/docs/builtins/pdf-templates`](/docs/builtins/pdf-templates). Element and option reference: [PDF & Factur-X](pdf.md).

## Choosing one

| Template | VAT breakdown | Payment QR | Signature box | Grouped subtotals | Conditional lines | Chart | Factur-X |
|---|---|---|---|---|---|---|---|
| `invoice_compliant` | ✓ | ✓ | — | — | — | — | ✓ |
| `invoice_minimal` | — | — | — | — | — | — | — |
| `invoice_subscription` | — | — | — | — | — | ✓ | — |
| `credit_note` | ✓ | — | — | — | — | — | ✓ |
| `quote_sections` | — | — | ✓ | ✓ | ✓ | — | — |
| `quote_options` | — | — | ✓ | — | — | — | — |
| `invoice` (starter) | — | — | — | — | — | — | ✓ |
| `quote` (starter) | — | — | — | — | — | — | — |

"Factur-X" marks the templates whose data carries everything the EN 16931 profile needs. Any template can be rendered as Factur-X — see [Rendering as Factur-X](#rendering-as-factur-x).

## The templates

### `invoice_compliant` — compliance is the content

Everything a VAT invoice must carry, laid out so an auditor finds it fast: both parties' VAT numbers, supply date alongside issue date, a **VAT breakdown by rate**, the statutory late-payment terms, and an EPC QR that pre-fills the transfer in a banking app. No colour band — ink-blue rules do the work, so it reads as a form rather than a brochure.

The breakdown is a second data-bound table over a `vat_breakdown` array, one row per rate:

```json
{ "type": "table", "data": "vat_breakdown",
  "header_columns": [],
  "rows": [ [
    { "text": "${rate}",   "width": 90,  "alignment": "left"  },
    { "text": "${base}",   "width": 110, "alignment": "right" },
    { "text": "${amount}", "width": 100, "alignment": "right" },
    { "text": "",          "width": 181 }
  ] ] }
```

### `invoice_minimal` — the amount due *is* the document

One number at 46 pt, everything else at 8 pt, two rules in the whole page, and no accent colour at all. The hierarchy is carried entirely by the type scale.

It widens `options.margins` to 64, which narrows the content width to **467 pt** — every column width in the file adds up to that instead of the default 481.

```json
{ "options": { "margins": 64 } }
```

### `invoice_subscription` — a billing period, not a list of deliverables

The period band comes first because it is the thing customers query. Plan fees (billed in advance) are separated from metered usage (billed in arrears), the prorated seat carries its own explanatory sub-line, and a `donut` chart shows spend by product.

Chart values must be real JSON numbers, not formatted strings:

```json
{ "type": "chart", "kind": "donut",
  "data": "spend", "label": "name", "value": "amount",
  "width": 210, "height": 106, "legend": true,
  "colors": ["312E81", "5B54C4", "8B84E8", "C3BEF6"] }
```

### `credit_note` — a cancellation has to be traceable

A band under the header names the invoice being corrected, its date, its original amount and the reason. Amounts are negative throughout. It credits `invoice_compliant`, so the two read as one sequence.

The solid total panel is a `rect` with `spans` text drawn over it. Cell `content` arrays have **no `color` field**, so white-on-colour has to be done this way — and `rect` not advancing the cursor is what makes the overlay work:

```json
{ "type": "move", "x": 271, "y": 2 },
{ "type": "rect", "width": 210, "height": 58, "fill": "9F1239" },
{ "type": "move", "x": 16, "y": 12 },
{ "type": "paragraph", "spans": [
    { "text": "TOTAL CREDITED", "fontWeight": "bold", "color": "F6D3DC" } ],
  "options": { "fontSize": 8 } }
```

### `quote_sections` — the work reads by section

Line items grouped into numbered trade sections, each closing with its own subtotal, so a client can approve or challenge one trade without unpicking the whole figure. Ends in a dashed **acceptance box** with date and signature rules.

> **Grouping recipe.** A data-bound `table` or a nested `repeat` **cannot** resolve a relative path against the item it sits inside: `"data": "lines"` nested inside a `repeat` over `sections` silently renders nothing. Flatten the group in your data instead — one array, each row tagged with a `kind` — and branch on it with `if`.

```json
{ "type": "repeat", "data": "rows", "content": [
  { "type": "if", "when": "kind", "equals": "section",
    "content": [ /* tinted full-width band */ ],
    "else": [
      { "type": "if", "when": "kind", "equals": "subtotal",
        "content": [ /* right-aligned rule + bold figure */ ],
        "else":    [ /* ordinary description / qty / unit / amount row */ ] } ] } ] }
```

```json
"rows": [
  { "kind": "section",  "name": "01   STRUCTURAL WORK" },
  { "kind": "line",     "name": "Demolition of partition wall", "qty": "1", "unit": "850.00", "amount": "850.00" },
  { "kind": "subtotal", "name": "Subtotal, structural work", "amount": "1,402.00" }
]
```

### `quote_options` — the client chooses

Base scope and optional work are separated, each priced independently, and the page closes on **two totals** side by side: base only, and with everything ticked.

The tick box is an empty cell with all four borders on — no glyph involved. That matters: none of the bundled fonts cover `☐` or `□`, so a box character renders as tofu.

```json
{ "text": "", "width": 22, "valign": "middle",
  "borderSides": { "top": "true", "left": "true", "right": "true", "bottom": "true" },
  "borderColor": "4C3A8A" }
```

### `invoice` and `quote` — the starter pair

A teal identity band, a billed-to block, a data-bound items table and a totals stack, in about 130 lines with nothing clever in it. Start here if none of the structural models match, and add to it. `quote` is the same skeleton with a different identity and a "valid until" strip.

## Using a template

```soli
def show
  let invoice = Invoice.find(params["id"])

  # The template is static; only the data changes per invoice.
  let template = slurp("pdf/invoice_compliant.template.json")
  let data     = { "data": invoice.to_render_hash() }.to_json()

  return pdf_response(template, data, {
    "filename": "invoice-#{invoice.number}.pdf",
    "title": "Invoice #{invoice.number}"
  })
end
```

The data document may be wrapped in `{ "data": { … } }` or passed as a bare object — both are accepted. Placeholders that find no value render empty and log a warning, so a missing field degrades rather than fails.

## Rendering as Factur-X

Factur-X (EN 16931) embeds a machine-readable CII XML invoice inside a PDF/A-3b file. There are two routes, and the difference matters.

**You bring the XML** — your template, your data, your XML. Full control of the layout, which is what a compliance template needs:

```soli
let pdf = pdf_facturx(template, data, xml, { "profile": "en16931" })
```

**The engine computes it** — give it a typed invoice and it derives the totals, the VAT breakdown *and* the XML:

```soli
let pdf = pdf_facturx_from_invoice(template, invoice, { "profile": "en16931" })
```

> **The typed-invoice route uses a different placeholder namespace.** `pdf_facturx_from_invoice` ignores your data file and builds its own, so a template written for `pdf_render` will not interpolate against it. It supplies `invoice.*` (`number`, `created_at`, `due_date`, `due_amount`, `payment_terms`, `type_label`), `company.*` and `customer.*` (`name`, `address`, `zipcode`, `city`, `country`, `phone`), `items[]`, `discounts[]`, `charges[]`, `total.*`, `infos.text` and `payment.*`.
>
> It does **not** expose party VAT identifiers or a per-rate VAT breakdown array. If your template must print those — as `invoice_compliant` does — use `pdf_facturx` and supply the XML yourself. A ready CII XML (`invoice_compliant.facturx.xml`) and a typed invoice (`invoice_compliant.invoice.json`) are both included so you can compare the two routes.

Both routes reject the `password` and `pdfa` options — the output is already PDF/A-3b.

## Regenerating the previews

The images on the rendered page are committed artifacts under `www/public/images/docs/pdf/`. After editing a template:

```bash
scripts/gen_pdf_previews.sh                     # every sample
scripts/gen_pdf_previews.sh invoice_minimal     # just one
```

Needs `pdftoppm` (poppler-utils). Renders page 1 of each sample at 150 DPI, which for A4 gives 1240×1755.

## Layout notes

These apply to any template you build on top of these:

- **Layout is tables.** There is no positioning primitive — header bands, meta grids, totals stacks and signature boxes are all `table` elements with `borderSides` set to `"false"`. Column widths in a row must sum to the content width (481 pt at default margins).
- `rect`, `ellipse`, `line`, `qr`, `barcode` and `image` **do not advance the cursor**; follow each with a `move`. Negative `y` moves up.
- `rect` border width is `borderWidth` (camelCase). `receipt.template.json` writes `border_width`, which is silently ignored — don't copy that.
- Table options mix conventions: `header.fillColor` is camelCase, `padding_x` / `padding_y` are snake_case.
- `borderSides` absent entirely means no borders; present means omitted sides default to `true`.
- Colours are hex **without** `#`. Only Titillium, JetBrains Mono and Noto Sans JP ship in `font/`.
