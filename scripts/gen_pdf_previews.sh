#!/usr/bin/env bash
# Render the PDF samples in www/public/pdf-samples/ and rasterise page 1 of
# each to www/public/images/docs/pdf/<name>.png for the documentation site.
#
# The PNGs are committed artifacts — the docs pages reference them directly.
# Re-run this after editing any sample template or data file.
#
#   scripts/gen_pdf_previews.sh              # regenerate every sample
#   scripts/gen_pdf_previews.sh invoice_minimal credit_note
#
# The work happens in scripts/gen_pdf_previews.sl. This wrapper only pins the
# working directory to the repository root and forwards the sample filter.
#
# It no longer needs poppler (`pdftoppm`/`pdfinfo`), python3, or a separate
# cargo build of the pdf/ workspace: `soli` rasterises its own pages.
set -euo pipefail

cd "$(dirname "$0")/.."

if ! command -v soli >/dev/null 2>&1; then
    echo "error: soli not found on PATH — build it with 'rbuild lang' or 'cargo install --path . --locked'" >&2
    exit 1
fi

# The script has no way to set an exit code, so its output carries the verdict.
output="$(SAMPLES="$*" soli scripts/gen_pdf_previews.sl)"
echo "$output"
grep -q '^FAIL ' <<<"$output" && exit 1
exit 0
