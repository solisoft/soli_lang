#!/usr/bin/env bash
# scripts/build-edge.sh — build the edge runtime: Soli compiled to wasm32 for
# Cloudflare Workers.
#
# Produces, in $OUT (default: target/edge):
#   soli_edge_bg.wasm   the interpreter, size-optimised (opt-level z, fat LTO)
#   soli_edge.js        wasm-bindgen glue (--target web)
#   jspi.js, worker.js  the Worker host, copied from edge/js
#
# `soli edge build` packages that directory with an app; point it there with
# --runtime or SOLI_EDGE_RUNTIME.
#
# Needs: rustup target wasm32-unknown-unknown, a C-free toolchain (nothing in
# the wasm build compiles C), and the wasm-bindgen CLI at the version
# edge/Cargo.toml pins:  cargo install wasm-bindgen-cli --version <it> --locked
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${OUT:-$ROOT/target/edge}"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"

want=$(sed -n 's/^wasm-bindgen = "=\(.*\)"$/\1/p' "$ROOT/edge/Cargo.toml")
have=$(wasm-bindgen --version 2>/dev/null | awk '{print $2}' || true)
if [[ "$have" != "$want" ]]; then
  echo "wasm-bindgen CLI ${have:-missing}, edge/Cargo.toml pins $want:" >&2
  echo "  cargo install wasm-bindgen-cli --version $want --locked" >&2
  exit 1
fi
rustup target list --installed 2>/dev/null | grep -qx wasm32-unknown-unknown || {
  echo "missing target: rustup target add wasm32-unknown-unknown" >&2
  exit 1
}

# getrandom 0.3 picks its JS backend from a cfg, not a feature.
RUSTFLAGS="${RUSTFLAGS:-} --cfg getrandom_backend=\"wasm_js\"" \
  cargo build --release --locked --target wasm32-unknown-unknown \
  --manifest-path "$ROOT/edge/Cargo.toml" --target-dir "$TARGET_DIR"

mkdir -p "$OUT"
wasm-bindgen --target web --out-dir "$OUT" \
  "$TARGET_DIR/wasm32-unknown-unknown/release/soli_edge.wasm"
cp "$ROOT/edge/js/jspi.js" "$ROOT/edge/js/worker.js" "$OUT/"
rm -f "$OUT"/*.d.ts

wasm="$OUT/soli_edge_bg.wasm"
printf 'edge runtime in %s: %s bytes, %s gzipped\n' "$OUT" \
  "$(wc -c < "$wasm")" "$(gzip -9 -c "$wasm" | wc -c)"
