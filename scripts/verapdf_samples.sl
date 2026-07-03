# Generate PDFs for veraPDF conformance checking in CI (see .github/workflows/ci.yml).
# The tagged + pdfa document must validate as BOTH PDF/A-3b and PDF/UA-1 —
# guarding the accessible+archival composition against regressions.

acc = slurp("www/public/pdf-samples/accessible.template.json")
tagged = pdf_render(acc, "{}", {
  "font_dirs": ["font"],
  "pdfa": true,
  "title": "Accessibility Report"
})
file_write_base64("/tmp/vp_tagged_pdfa.pdf", tagged)
print("wrote /tmp/vp_tagged_pdfa.pdf (#{tagged.length} b64 chars)")
