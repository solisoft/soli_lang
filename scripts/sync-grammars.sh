#!/usr/bin/env bash
# Copy the canonical TextMate grammars in editors/grammars/ into the editor
# bundles that ship them. With --check, change nothing and fail when a copy
# differs from its source (CI runs it that way).
#
#   scripts/sync-grammars.sh           # copy
#   scripts/sync-grammars.sh --check   # verify only
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
check=0
if [[ "${1:-}" == "--check" ]]; then
  check=1
elif [[ $# -gt 0 ]]; then
  echo "usage: $0 [--check]" >&2
  exit 2
fi

# source -> destination, relative to the repository root.
copies=(
  "editors/grammars/soli.tmLanguage.json editors/vscode/syntaxes/soli.tmLanguage.json"
  "editors/grammars/soli-template.tmLanguage.json editors/vscode/syntaxes/soli-template.tmLanguage.json"
  "editors/grammars/soli.tmLanguage.json editors/nova/soli.novaextension/Syntaxes/soli.tmLanguage.json"
)

stale=0
for pair in "${copies[@]}"; do
  read -r src dst <<<"$pair"
  if [[ $check -eq 1 ]]; then
    if ! cmp -s "$root/$src" "$root/$dst"; then
      echo "out of date: $dst (run scripts/sync-grammars.sh)" >&2
      stale=1
    fi
  else
    mkdir -p "$(dirname "$root/$dst")"
    cp "$root/$src" "$root/$dst"
    echo "synced $dst"
  fi
done

exit $stale
