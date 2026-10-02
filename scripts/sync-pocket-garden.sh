#!/usr/bin/env bash
# The docs site plays Pocket Garden inside its blog post
# (www/docs/blog/pocket-garden-eui.md), so it serves the game itself and
# carries copies of two things:
#
#   examples/pocket-garden/*.sl        -> www/app/controllers/pocket_garden/
#     the game, without garden.sl (which opens a window);
#   src/scaffold/templates/eui/eui_builders*.sl -> www/app/controllers/
#     the widget catalogue `soli new --eui` scaffolds, which the game's
#     views call.
#
# Copies, not symlinks: an import may not resolve outside the project
# (SEC-076), symlinks included, and the deploy rsyncs www/ alone.
#
#   scripts/sync-pocket-garden.sh          write the copies
#   scripts/sync-pocket-garden.sh --check  fail if a copy has drifted
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
check=false
[ "${1:-}" = "--check" ] && check=true

pairs=()
for name in species beds shop herbarium game; do
  pairs+=("examples/pocket-garden/$name.sl" "www/app/controllers/pocket_garden/$name.sl")
done
for source in "$root"/src/scaffold/templates/eui/eui_builders*.sl; do
  file="$(basename "$source")"
  pairs+=("src/scaffold/templates/eui/$file" "www/app/controllers/$file")
done

drifted=0
for ((i = 0; i < ${#pairs[@]}; i += 2)); do
  source="$root/${pairs[i]}"
  copy="$root/${pairs[i + 1]}"
  if $check; then
    if ! cmp -s "$source" "$copy"; then
      echo "drifted: ${pairs[i + 1]} differs from ${pairs[i]}" >&2
      drifted=1
    fi
  else
    mkdir -p "$(dirname "$copy")"
    cp "$source" "$copy"
  fi
done

if $check && [ "$drifted" -ne 0 ]; then
  echo "Run scripts/sync-pocket-garden.sh and commit the copies." >&2
  exit 1
fi
$check && echo "the docs site's Pocket Garden matches its sources"
exit 0
