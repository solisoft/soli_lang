#!/usr/bin/env bash
# Parse the repository's Soli code with the grammar and report how many files
# parse without an error. Fails when fewer than MIN_PERCENT (default 95) do.
#
#   editors/tree-sitter-soli/script/corpus.sh            # summary
#   editors/tree-sitter-soli/script/corpus.sh --list     # and the failing files
set -euo pipefail

here="$(cd "$(dirname "$0")/.." && pwd)"
root="$(cd "$here/../.." && pwd)"
min="${MIN_PERCENT:-95}"
read -ra tree_sitter <<< "${TREE_SITTER:-tree-sitter}"

list="$(mktemp)"
out="$(mktemp)"
trap 'rm -f "$list" "$out"' EXIT

find "$root/tests" "$root/examples" "$root/www/app" "$root/www/config" "$root/src/scaffold" \
  -name '*.sl' -not -path '*/node_modules/*' | sort > "$list"

cd "$here"
"${tree_sitter[@]}" parse --quiet --stat --paths "$list" > "$out" 2>&1 || true

total="$(wc -l < "$list" | tr -d ' ')"
failed="$(grep -cE '\((ERROR|MISSING)' "$out" || true)"
ok=$((total - failed))
percent=$((ok * 100 / total))

echo "tree-sitter-soli: $ok of $total files parse without an error ($percent%, minimum $min%)"
if [ "${1:-}" = "--list" ]; then
  grep -E '\((ERROR|MISSING)' "$out" | sed "s|$root/||" | awk '{print "  " $1}' || true
fi
[ "$percent" -ge "$min" ]
