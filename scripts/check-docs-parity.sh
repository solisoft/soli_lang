#!/bin/bash
# Report topics that exist on one documentation surface and not the other.
#
# The repository's policy (root `CLAUDE.md`) asks for every user-facing topic to
# exist as markdown in `www/docs/` *and* as a hand-maintained page under
# `www/app/views/docs/`. Neither is generated from the other, so both drift, and
# it has drifted in both directions — `core-concepts/streaming.html.slv` was a
# routed 11 kB page with no markdown source for as long as it existed.
#
# A missing markdown file costs more than it looks: `soli new` bundles the whole
# of `www/docs/` into each new application under `docs/`, so a topic with no
# markdown is absent from every application anyone creates.
#
#   ./scripts/check-docs-parity.sh
#
# Advisory, not a gate. The allowlists below *are* the judgement; this only
# remembers them. See `www/docs/CLAUDE.md` for how to decide which list a new
# file belongs in.
set -u
cd "$(dirname "$0")/.."

# One markdown file backing a whole `.slv` section rather than a single page.
HUBS=(
  builtins          # → www/app/views/docs/builtins/*        (~40 pages)
  soli-language     # → www/app/views/docs/language/*        (22 pages)
  database          # → www/app/views/docs/database/*
)

# Ships in applications under `docs/`, deliberately not on the site. Each of
# these carries a `> **Markdown-only, deliberately.**` note saying why.
MD_ONLY=(
  testing-e2e
  testing-assertions
  solidb-reference
  testing-guide     # superseded by testing-e2e.md; see tasks/todo/
)

in_list() { local n=$1; shift; for e in "$@"; do [ "$e" = "$n" ] && return 0; done; return 1; }

unpaired_md=0
for f in www/docs/*.md; do
  base=$(basename "$f" .md)
  [ "$base" = "CLAUDE" ] && continue
  in_list "$base" "${HUBS[@]}"    && continue
  in_list "$base" "${MD_ONLY[@]}" && continue
  # a page anywhere under the docs views with this name counts as paired
  if ! find www/app/views/docs \( -name "${base}.html.slv" -o -name "${base//-/_}.html.slv" \) \
       | grep -q .; then
    echo "  markdown with no page: $f"
    unpaired_md=$((unpaired_md + 1))
  fi
done

# Pages that are the site itself rather than a topic, so there is nothing to
# ship inside an application. Short list, and it should stay short.
SLV_ONLY=(
  changelog        # the release history of the site's own project
  comparison       # "How Soli Compares" — an argument, not a reference
  pdf_editor       # interactive tool: a canvas and its JavaScript
  pdf_studio       # interactive tool
  pdf_playground   # interactive tool
)

# The other direction. Anything not in SLV_ONLY is a gap: the markdown is what
# ships inside applications, so a topic with none is missing from all of them.
unpaired_slv=0
while IFS= read -r p; do
  base=$(basename "$p" .html.slv)
  # section pages are backed by a hub, and index pages back nothing
  case "$p" in
    */docs/builtins/*|*/docs/language/*|*/docs/database/*) continue ;;
  esac
  [ "$base" = "index" ] && continue
  in_list "$base" "${SLV_ONLY[@]}" && continue
  # the markdown may live in a subdirectory of its own (www/docs/eui/, native/)
  if ! find www/docs -name "${base}.md" -o -name "${base//_/-}.md" | grep -q .; then
    echo "  page with no markdown: $p"
    unpaired_slv=$((unpaired_slv + 1))
  fi
done < <(find www/app/views/docs -name '*.html.slv' | sort)

echo "--- $unpaired_md markdown file(s) with no page, $unpaired_slv page(s) with no markdown ---"
