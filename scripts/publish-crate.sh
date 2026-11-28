#!/usr/bin/env bash
# Publish `solilang` (and its `soli-yaml` dependency) to crates.io.
#
# The manifest cannot be published as it stands: crates.io refuses git and
# unversioned path dependencies, even optional ones, and drops `[patch]`.
# So this script stages HEAD in a temporary directory, rewrites the staged
# manifest, and publishes from there — the repo's Cargo.toml, which CI, the
# release binaries and the Docker image build, stays untouched.
#
# Left out of the crates.io build, because what they need is not on crates.io:
#   eui, eui-desktop  eui-proto / eui-client / eui-score (git)
#   es                es-client (git)
#   solidb-driver     solidb-client (git; crates.io's 0.7.0 is not that code)
#   pdf               soli-pdf, which vendors a patched printpdf
#   codegraph         soli-codegraph (path crate, not published)
#   office            left out while calamine 0.36 needed a vendored patch for
#                     zip 9; calamine 0.38 builds on zip 9 from crates.io, so
#                     it can come back once a dry run here confirms it
# Users who need them install the release binaries or the Docker image.
#
#   ./scripts/publish-crate.sh            # dry run: package and build
#   ./scripts/publish-crate.sh --publish  # upload (needs `cargo login`)
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
publish=false
[[ "${1:-}" == "--publish" ]] && publish=true

stage="$(mktemp -d "${TMPDIR:-/tmp}/solilang-publish.XXXXXX")"
trap 'rm -rf "$stage"' EXIT

# Only committed files: what gets published is what the tag points at.
git -C "$root" archive HEAD | tar -x -C "$stage"

python3 - "$stage/Cargo.toml" <<'PY'
import re, sys

path = sys.argv[1]
text = open(path).read()

dropped_features = ["codegraph", "solidb-driver", "office", "pdf", "eui", "es", "eui-desktop"]
dropped_deps = ["solidb-client", "soli-pdf", "soli-codegraph", "eui-proto", "eui-client",
                "eui-score", "es-client", "calamine", "cms"]

# Remove them from the `default` and `full` lists.
for feature in dropped_features:
    text = re.sub(rf'^    "{re.escape(feature)}",\n', "", text, flags=re.M)

lines = text.split("\n")
out = []
for line in lines:
    key = line.split("=", 1)[0].strip()
    if "=" in line and not line.startswith(("#", " ")):
        if key in dropped_features and line.lstrip().startswith(key) and "[" in line:
            continue
        if key in dropped_deps and "{" in line:
            continue
    out.append(line)
text = "\n".join(out)

text = text.replace('soli-yaml = { path = "yaml" }', 'soli-yaml = { path = "yaml", version = "0.1.0" }')

# `[patch]` is ignored on publish anyway; drop it so the staged build matches.
text = re.sub(r'^\[patch\.crates-io\]\n(?:[^\[\n].*\n|\n)*', "", text, flags=re.M)

open(path, "w").write(text)
PY

# soli-yaml is published from the staged copy too.
sed -i.bak '/^publish = false$/d' "$stage/yaml/Cargo.toml" && rm "$stage/yaml/Cargo.toml.bak"

# crates.io caps a crate at 10 MiB compressed, and the whole tree (the docs
# site, fonts, benchmarks) is three times that. Keep what the build reads:
# src/, the www/docs it embeds, bench targets, and the test-only x509 fixtures.
keep=" Cargo.toml Cargo.lock build.rs README.md LICENSE src benches yaml www tests "
for entry in "$stage"/* "$stage"/.[!.]*; do
    [[ -e "$entry" ]] || continue
    [[ "$keep" == *" $(basename "$entry") "* ]] || rm -rf "$entry"
done
find "$stage/www" -mindepth 1 -maxdepth 1 ! -name docs -exec rm -rf {} +
find "$stage/tests" -mindepth 1 -maxdepth 1 ! -name fixtures -exec rm -rf {} +

# A workspace member list naming removed crates would break the build.
if grep -q '^\[workspace\]' "$stage/Cargo.toml"; then
    echo "error: Cargo.toml has a [workspace]; update publish-crate.sh to rewrite it" >&2
    exit 1
fi

cd "$stage"
echo "Staged in $stage"
grep -nE 'git *=|path *= *"(pdf|codegraph|vendor)' Cargo.toml && {
    echo "error: staged manifest still has unpublishable dependencies" >&2
    exit 1
}

if $publish; then
    yaml_version="$(sed -n 's/^version = "\(.*\)"$/\1/p' yaml/Cargo.toml)"
    if curl -sf -A "solilang-publish" \
        "https://crates.io/api/v1/crates/soli-yaml/$yaml_version" > /dev/null; then
        echo "soli-yaml $yaml_version is already on crates.io"
    else
        (cd yaml && cargo publish --allow-dirty)
    fi
    cargo publish --allow-dirty
else
    (cd yaml && cargo publish --dry-run --allow-dirty)
    # soli-yaml is not on crates.io until the real publish, so the main crate
    # can only be packaged and built against the local copy here.
    cargo package --allow-dirty --no-verify --list > /dev/null
    cargo build --release
    echo "Dry run OK. Publish with: $0 --publish"
fi
