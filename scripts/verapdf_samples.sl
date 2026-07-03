# Generate a PDF for veraPDF conformance checking in CI (see .github/workflows/ci.yml).
# The tagged + pdfa document exercises headings, a list and a table, and must
# validate as BOTH PDF/A-3b and PDF/UA-1 — guarding the accessible+archival
# composition AND the real L/Table structure tagging against regressions.

tpl = [[
{
  "fonts": ["titillium"],
  "options": { "tagged": true, "lang": "en-US" },
  "content": [
    { "type": "paragraph", "value": "Accessibility Report",
      "options": { "fontSize": 20, "fontWeight": "bold", "bookmark": "Report", "bookmarkLevel": 1 } },
    { "type": "paragraph", "value": "This document validates as PDF/A-3b and PDF/UA-1." },
    { "type": "list", "items": ["First point", "Second point", "Third point"] },
    { "type": "table",
      "header_columns": [ {"text":"Region","width":150}, {"text":"Value","width":150} ],
      "rows": [
        [ {"text":"EU","width":150}, {"text":"1.2M","width":150} ],
        [ {"text":"US","width":150}, {"text":"0.9M","width":150} ]
      ] }
  ]
}
]]

pdf = pdf_render(tpl, "{}", { "font_dirs": ["font"], "pdfa": true, "title": "Accessibility Report" })
file_write_base64("/tmp/vp_tagged_pdfa.pdf", pdf)
print("wrote /tmp/vp_tagged_pdfa.pdf (#{pdf.length} b64 chars)")
