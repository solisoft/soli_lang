# Rasterise page 1 of every PDF sample in www/public/pdf-samples/ to
# www/public/images/docs/pdf/<name>.png for the documentation site.
#
# The PNGs are committed artifacts — the docs pages reference them directly.
# Run this through scripts/gen_pdf_previews.sh, which sets the working
# directory to the repository root.
#
# This used to shell out to poppler's `pdftoppm`. It no longer does: the
# layout engine rasterises its own pages (`pdf_preview`), so the only tool
# required is `soli` itself.
#
# 150 DPI is the established convention: A4 comes out 1240x1754.

const DPI = 150
const SAMPLES_DIR = "www/public/pdf-samples"
const OUT_DIR = "www/public/images/docs/pdf"

# `markdown` has no .template.json — it goes through the Markdown renderer,
# which `pdf_preview_from_markdown` now previews just the same.
const ALL_SAMPLES = [
    "invoice", "invoice_compliant", "invoice_minimal", "invoice_subscription",
    "quote", "quote_sections", "quote_options", "credit_note",
    "receipt", "statement", "letter", "report", "graphics", "dynamic",
    "accessible", "features", "markdown"
]

requested = getenv("SAMPLES").to_s.trim()
samples = ALL_SAMPLES
unless requested.blank?
    samples = requested.split(" ")
end

failures = 0

for name in samples
    options = {
        "dpi": DPI,
        "pages": [1],
        "font_dirs": ["font"],
        "out_dir": OUT_DIR,
        "prefix": name
    }

    try {
        paths = []
        if name == "markdown"
            paths = pdf_preview_from_markdown(slurp("#{SAMPLES_DIR}/markdown.md"), options)
        else
            template_path = "#{SAMPLES_DIR}/#{name}.template.json"
            if file_exists(template_path)
                data_path = "#{SAMPLES_DIR}/#{name}.data.json"
                data = "{}"
                data = slurp(data_path) if file_exists(data_path)
                paths = pdf_preview(slurp(template_path), data, options)
            end
        end

        if paths.length() == 0
            print("skip  #{name} (no template)")
        else
            image = Image.new(paths[0])
            print("ok    #{name}  (#{image.width}x#{image.height})")
        end
    } catch error {
        print("FAIL  #{name}: #{error}")
        failures = failures + 1
    }
end

# Soli scripts have no exit-code builtin, so the wrapper reads this line.
if failures > 0
    print("")
    print("#{failures} sample(s) failed")
end
