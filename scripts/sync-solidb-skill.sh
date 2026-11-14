#!/usr/bin/env bash
# `soli new` and `soli update docs` write the solidb skill into every app
# (.claude/skills/solidb/), so that an agent working there knows SDBQL and
# the SoliDB HTTP API as observed on a running server. The skill is written
# and kept current outside any repository, in ~/.claude/skills/solidb/ (the
# SoliDB repository's CLAUDE.md says when); what the binary embeds is a copy:
#
#   ~/.claude/skills/solidb/  ->  src/scaffold/templates/skills/solidb/
#
# Never edit the copy by hand: edit the skill, run this, commit the result.
#
#   scripts/sync-solidb-skill.sh          write the copy
#   scripts/sync-solidb-skill.sh --check  fail if the copy has drifted
#
# Where the skill is not installed (CI, another machine) there is nothing to
# compare with: --check passes, and a plain run fails.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
source="${SOLIDB_SKILL_DIR:-$HOME/.claude/skills/solidb}"
copy="$root/src/scaffold/templates/skills/solidb"

if [ ! -f "$source/SKILL.md" ]; then
  if [ "${1:-}" = "--check" ]; then
    echo "sync-solidb-skill: no skill at $source, nothing to compare"
    exit 0
  fi
  echo "sync-solidb-skill: no skill at $source (set SOLIDB_SKILL_DIR)" >&2
  exit 1
fi

if [ "${1:-}" = "--check" ]; then
  if diff -rq -x '._*' -x '.DS_Store' "$source" "$copy" >/dev/null 2>&1; then
    echo "sync-solidb-skill: the copy is current"
    exit 0
  fi
  echo "sync-solidb-skill: src/scaffold/templates/skills/solidb differs from $source:" >&2
  diff -rq -x '._*' -x '.DS_Store' "$source" "$copy" >&2 || true
  echo "run scripts/sync-solidb-skill.sh and commit the result" >&2
  exit 1
fi

mkdir -p "$copy"
# `._*` and `.DS_Store`: what macOS leaves beside files it has touched.
rsync -a --delete --delete-excluded --exclude '._*' --exclude '.DS_Store' "$source/" "$copy/"
echo "sync-solidb-skill: copied $(find "$copy" -type f | wc -l) files from $source"
