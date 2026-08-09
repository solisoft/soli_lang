#!/usr/bin/env bash
set -euo pipefail

# Usage: ./scripts/release.sh <major|minor|patch> [--dry-run]
#
# Bumps the version in Cargo.toml, commits, tags, and pushes.
# The CI will then build binaries and create the GitHub release.

DRY_RUN=false
BUMP_TYPE="${1:-}"

for arg in "$@"; do
  if [ "$arg" = "--dry-run" ]; then
    DRY_RUN=true
  fi
done

if [[ -z "$BUMP_TYPE" || ! "$BUMP_TYPE" =~ ^(major|minor|patch)$ ]]; then
  echo "Usage: $0 <major|minor|patch> [--dry-run]"
  exit 1
fi

# Ensure clean working tree
if [ -n "$(git status --porcelain)" ]; then
  echo "Error: working tree is not clean. Commit or stash changes first."
  exit 1
fi

# Ensure we're on main
BRANCH=$(git branch --show-current)
if [ "$BRANCH" != "main" ]; then
  echo "Error: releases must be made from main (currently on '$BRANCH')"
  exit 1
fi

# Get current version from Cargo.toml
CURRENT=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
IFS='.' read -r MAJOR MINOR PATCH <<< "$CURRENT"

case "$BUMP_TYPE" in
  major) MAJOR=$((MAJOR + 1)); MINOR=0; PATCH=0 ;;
  minor) MINOR=$((MINOR + 1)); PATCH=0 ;;
  patch) PATCH=$((PATCH + 1)) ;;
esac

NEW_VERSION="${MAJOR}.${MINOR}.${PATCH}"
TAG="v${NEW_VERSION}"

echo "Current version: $CURRENT"
echo "New version:     $NEW_VERSION"
echo "Tag:             $TAG"

if [ "$DRY_RUN" = true ]; then
  echo "(dry run — no changes made)"
  exit 0
fi

# Update Cargo.toml (portable: GNU sed -i and BSD sed -i '' both work via temp file)
python3 - "$CURRENT" "$NEW_VERSION" <<'PY'
import re, sys
from pathlib import Path
current, new = sys.argv[1], sys.argv[2]
path = Path("Cargo.toml")
text = path.read_text()
updated, n = re.subn(
    rf'^version = "{re.escape(current)}"',
    f'version = "{new}"',
    text,
    count=1,
    flags=re.M,
)
if n != 1:
    sys.exit(f"Error: expected one version = \"{current}\" in Cargo.toml, found {n}")
path.write_text(updated)
print(f"Cargo.toml -> {new}")
PY

# Update Cargo.lock package version for solilang
cargo check --quiet 2>/dev/null || true

# Commit and tag
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to v${NEW_VERSION}"
git tag -a "$TAG" -m "Release $TAG"

# Push commit and tag together
git push && git push origin "$TAG"

echo ""
echo "Released $TAG — CI will build binaries and create the GitHub release."
