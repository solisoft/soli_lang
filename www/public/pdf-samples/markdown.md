# Quarterly Report

**Q3 2026** — revenue was **up 12%**, driven by *strong* EU growth and the new
`pdf_from_markdown` builtin. No template to author — this whole page is one
Markdown string rendered by the layout engine.

## Highlights

- Signed e-invoices (PAdES-B-B and **B-T** timestamps)
- Markdown → designed PDF
  - nested bullets work
  - so do ordered lists
- Accessible **and** archival output in one file

## Numbers

| Region | Revenue | Growth |
|--------|---------|--------|
| EU     | 1.20M   | +18%   |
| US     | 0.90M   | +7%    |
| APAC   | 0.40M   | +25%   |

## How it works

Write Markdown, get a designed PDF:

```
let pdf = pdf_from_markdown(md, { headingColor: "0f766e" })
file_write_base64("report.pdf", pdf)
```

Headings, ~~struck text~~, `inline code`, [links](https://soli.dev/docs) and
images all map onto the engine's elements.

> Tip: pass a theme — `fonts`, `fontSize`, `lineHeight`, `headingColor`,
> `linkColor`, `codeColor` — to restyle without touching the content.

---

Generated from Markdown by the Soli PDF engine.
