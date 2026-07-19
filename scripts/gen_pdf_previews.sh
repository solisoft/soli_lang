#!/usr/bin/env bash
# Render the PDF samples in www/public/pdf-samples/ and rasterise page 1 of each
# to www/public/images/docs/pdf/<name>.png for the documentation site.
#
# The PNGs are committed artifacts — the docs pages reference them directly.
# Re-run this after editing any sample template or data file.
#
#   scripts/gen_pdf_previews.sh              # regenerate every sample
#   scripts/gen_pdf_previews.sh invoice_minimal credit_note
#
# 150 DPI is the established convention: A4 rasterises to 1240x1755, matching
# the previews that were already in the repo.
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$PWD"

SAMPLES_DIR="www/public/pdf-samples"
OUT_DIR="www/public/images/docs/pdf"
RENDER_BIN="pdf/target/release/render_pdf"
DPI=150

if ! command -v pdftoppm >/dev/null 2>&1; then
    echo "error: pdftoppm not found — install poppler-utils" >&2
    echo "  Debian/Ubuntu: sudo apt install poppler-utils" >&2
    echo "  macOS:         brew install poppler" >&2
    exit 1
fi

if [ ! -x "$RENDER_BIN" ]; then
    echo "building $RENDER_BIN ..."
    # pdf/ is its own cargo workspace, not a member of the root manifest.
    (cd pdf && cargo build --release --bin render_pdf)
fi

# The full sample set. `markdown` is excluded: it goes through pdf_from_markdown
# rather than the template renderer, so it has no .template.json.
ALL_SAMPLES=(
    invoice invoice_compliant invoice_minimal invoice_subscription
    quote quote_sections quote_options credit_note
    receipt statement letter report graphics dynamic accessible features
)

if [ "$#" -gt 0 ]; then
    SAMPLES=("$@")
else
    SAMPLES=("${ALL_SAMPLES[@]}")
fi

mkdir -p "$OUT_DIR"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

fail=0
for name in "${SAMPLES[@]}"; do
    template="$SAMPLES_DIR/$name.template.json"
    data="$SAMPLES_DIR/$name.data.json"

    if [ ! -f "$template" ]; then
        echo "skip  $name (no $template)"
        continue
    fi
    [ -f "$data" ] || data="/dev/null"

    if ! "$RENDER_BIN" --template "$template" --data "$data" \
            --font-dir "$ROOT/font" -o "$tmp/$name.pdf" >/dev/null 2>"$tmp/$name.err"; then
        echo "FAIL  $name" >&2
        sed 's/^/        /' "$tmp/$name.err" >&2
        fail=1
        continue
    fi

    # Page 1 only — the gallery shows a single representative sheet.
    # pdftoppm zero-pads the page suffix to the page count's width, so a
    # 12-page document yields "-01" rather than "-1": glob instead of guessing.
    pdftoppm -png -r "$DPI" -f 1 -l 1 "$tmp/$name.pdf" "$tmp/$name-page"
    mv "$tmp/$name-page"-*.png "$OUT_DIR/$name.png"

    pages=$(pdfinfo "$tmp/$name.pdf" | awk '/^Pages:/ {print $2}')
    size=$(python3 -c "
import struct,sys
d=open('$OUT_DIR/$name.png','rb').read(33)
w,h=struct.unpack('>II',d[16:24])
print(f'{w}x{h}')")
    echo "ok    $name  ($size, ${pages}p)"
done

exit "$fail"
