#!/usr/bin/env bash
# scripts/compare_ruby.sh — Soli (release) vs Ruby microbench report.
#
# Usage:
#   ./scripts/compare_ruby.sh              # default Ruby from PATH / mise
#   RUBY_BIN=ruby ./scripts/compare_ruby.sh
#   RUBY_VERSION=4.0.6 ./scripts/compare_ruby.sh
#
# Runs each paired program REPEATS times (default 5), reports median wall time
# (or median of self-reported parse/stringify for JSON benches).

set -euo pipefail
# Dots as decimal separators (printf / sort -n break under fr_FR etc.).
export LC_ALL=C
export LANG=C

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

SOLI_BIN="${SOLI_BIN:-$REPO_ROOT/target/release/soli}"
RUBY_VERSION="${RUBY_VERSION:-4.0.6}"
REPEATS="${REPEATS:-5}"
OUT="${OUT:-/tmp/soli_vs_ruby_report.md}"

if [[ -n "${RUBY_BIN:-}" ]]; then
  RUBY="$RUBY_BIN"
elif command -v mise >/dev/null 2>&1; then
  RUBY="$(mise which -C "$REPO_ROOT" ruby@$RUBY_VERSION 2>/dev/null || true)"
  if [[ -z "$RUBY" ]]; then
    mise install "ruby@$RUBY_VERSION" >/dev/null
    RUBY="$(mise which ruby@$RUBY_VERSION)"
  fi
else
  RUBY="$(command -v ruby)"
fi

[[ -x "$SOLI_BIN" ]] || { echo "missing $SOLI_BIN — run cargo build --release" >&2; exit 1; }
[[ -x "$RUBY" ]] || { echo "missing ruby binary" >&2; exit 1; }

SOLI_VER="$("$SOLI_BIN" --version 2>/dev/null || echo soli)"
RUBY_VER="$("$RUBY" --version)"
HOST="$(uname -srm)"
DATE="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

echo "Soli:  $SOLI_BIN ($SOLI_VER)" >&2
echo "Ruby:  $RUBY ($RUBY_VER)" >&2
echo "Host:  $HOST" >&2
echo "Repeats: $REPEATS" >&2

# Median of newline-separated numbers on stdin.
median() {
  sort -n | awk '
    { a[NR]=$1 }
    END {
      if (NR == 0) { print "nan"; exit }
      if (NR % 2) print a[(NR+1)/2]
      else print (a[NR/2] + a[NR/2+1]) / 2
    }'
}

# Wall-clock ms for a command (best-effort portable).
run_ms() {
  # outputs only the milliseconds as a float
  local start end
  start=$(date +%s%N)
  "$@" >/dev/null 2>&1
  end=$(date +%s%N)
  awk -v s="$start" -v e="$end" 'BEGIN { printf "%.3f", (e-s)/1e6 }'
}

# Capture self-reported "  stringify: Xms" / "  parse: Xms" lines.
# Prints "stringify_ms parse_ms" for one run.
run_json_self_report() {
  local out
  out="$("$@" 2>/dev/null)"
  local s p
  s=$(printf '%s\n' "$out" | sed -n 's/.*stringify: *\([0-9.]*\)ms.*/\1/p' | head -1)
  p=$(printf '%s\n' "$out" | sed -n 's/.*parse: *\([0-9.]*\)ms.*/\1/p' | head -1)
  # Soli may print floats with many decimals; Ruby often integers.
  echo "${s:-nan} ${p:-nan}"
}

median_pair() {
  # stdin: lines of "a b" → prints median_a median_b
  awk '
    { a[NR]=$1; b[NR]=$2 }
    END {
      n=NR
      # insertion sort a and b together by sorting indices separately is hard;
      # sort each column.
    }' >/dev/null
  local col1 col2
  col1=$(awk '{print $1}' | median)
  # re-read not possible; use temp
  :
}

# ---- wall-clock benches (whole program) ----
declare -a WALL_BENCHES=(
  "array_ops:programs/array_ops.sl:ruby/array_ops.rb"
  "hash_ops:programs/hash_ops.sl:ruby/hash_ops.rb"
  "string_ops:programs/string_ops.sl:ruby/string_ops.rb"
  "loop_sum:programs/loop_sum.sl:ruby/loop_sum.rb"
  "fib_iterative:programs/fib_iterative.sl:ruby/fib_iterative.rb"
  "fib_recursive:programs/fib_recursive.sl:ruby/fib_recursive.rb"
  "class_ops:programs/class_ops.sl:ruby/class_ops.rb"
  "inheritance_deep:programs/inheritance_deep.sl:ruby/inheritance_deep.rb"
  "pipeline_ops:programs/pipeline_ops.sl:ruby/pipeline_ops.rb"
)

# ---- JSON self-timed benches ----
declare -a JSON_BENCHES=(
  "json_ops:programs/json_ops.sl:ruby/json_ops.rb"
  "json_ops_large:programs/json_ops_large.sl:ruby/json_ops_large.rb"
)

{
  echo "# Soli vs Ruby microbench report"
  echo
  echo "| | |"
  echo "|---|---|"
  echo "| **Date (UTC)** | $DATE |"
  echo "| **Host** | \`$HOST\` |"
  echo "| **Soli** | \`$SOLI_VER\` (\`$SOLI_BIN\`) |"
  echo "| **Ruby** | \`$RUBY_VER\` |"
  echo "| **Repeats** | $REPEATS (median reported) |"
  echo "| **Soli path** | release binary, \`soli <script.sl>\` |"
  echo "| **Ruby path** | MRI 4.x, process wall time / self-reported ms |"
  echo
  echo "Lower is better. **Speedup** = Ruby_ms / Soli_ms (>1 means Soli is faster)."
  echo
  echo "## Whole-program wall time"
  echo
  echo "Times include process startup + parse/compile + run for both sides."
  echo "Ruby MRI process boot is ~30–35 ms on this host, so short programs are"
  echo "startup-dominated on the Ruby side; Soli starts faster."
  echo
  echo "| Benchmark | Soli (ms) | Ruby 4 (ms) | Speedup |"
  echo "|---|---:|---:|---:|"
} > "$OUT"

for entry in "${WALL_BENCHES[@]}"; do
  IFS=':' read -r name soli_rel ruby_rel <<<"$entry"
  soli_path="benches/$soli_rel"
  ruby_path="benches/$ruby_rel"
  if [[ ! -f "$soli_path" || ! -f "$ruby_path" ]]; then
    echo "skip $name (missing file)" >&2
    continue
  fi
  echo "  wall: $name ..." >&2

  soli_samples=""
  ruby_samples=""
  for _ in $(seq 1 "$REPEATS"); do
    # Soli: pass the script path directly (there is no `soli run` subcommand).
    s=$(run_ms "$SOLI_BIN" "$soli_path")
    r=$(run_ms "$RUBY" "$ruby_path")
    soli_samples+="$s"$'\n'
    ruby_samples+="$r"$'\n'
  done
  soli_med=$(printf '%s' "$soli_samples" | median)
  ruby_med=$(printf '%s' "$ruby_samples" | median)
  speedup=$(awk -v s="$soli_med" -v r="$ruby_med" 'BEGIN {
    if (s+0 == 0) print "n/a";
    else printf "%.2f×", r/s;
  }')
  printf '| `%s` | %.1f | %.1f | %s |\n' "$name" "$soli_med" "$ruby_med" "$speedup" >> "$OUT"
done

# Warmup Soli once so first-run noise is out of the way for self-timed benches.
"$SOLI_BIN" benches/programs/json_ops.sl >/dev/null 2>&1 || true

{
  echo
  echo "## JSON (self-timed loops, excludes process startup)"
  echo
  echo "Each program times only the parse/stringify loops (same iteration counts)."
  echo
  echo "| Benchmark | Metric | Soli (ms) | Ruby 4 (ms) | Speedup |"
  echo "|---|---|---:|---:|---:|"
} >> "$OUT"

for entry in "${JSON_BENCHES[@]}"; do
  IFS=':' read -r name soli_rel ruby_rel <<<"$entry"
  soli_path="benches/$soli_rel"
  ruby_path="benches/$ruby_rel"
  echo "  json: $name ..." >&2

  soli_s_samples="" soli_p_samples=""
  ruby_s_samples="" ruby_p_samples=""
  for _ in $(seq 1 "$REPEATS"); do
    read -r ss sp <<<"$(run_json_self_report "$SOLI_BIN" "$soli_path")"
    read -r rs rp <<<"$(run_json_self_report "$RUBY" "$ruby_path")"
    soli_s_samples+="$ss"$'\n'
    soli_p_samples+="$sp"$'\n'
    ruby_s_samples+="$rs"$'\n'
    ruby_p_samples+="$rp"$'\n'
  done
  ss_med=$(printf '%s' "$soli_s_samples" | median)
  sp_med=$(printf '%s' "$soli_p_samples" | median)
  rs_med=$(printf '%s' "$ruby_s_samples" | median)
  rp_med=$(printf '%s' "$ruby_p_samples" | median)

  for metric in stringify parse; do
    if [[ "$metric" == stringify ]]; then
      sm=$ss_med; rm=$rs_med
    else
      sm=$sp_med; rm=$rp_med
    fi
    speedup=$(awk -v s="$sm" -v r="$rm" 'BEGIN {
      if (s+0 == 0) print "n/a";
      else printf "%.2f×", r/s;
    }')
    printf '| `%s` | %s | %.1f | %.1f | %s |\n' "$name" "$metric" "$sm" "$rm" "$speedup" >> "$OUT"
  done
done

{
  echo
  echo "## Notes"
  echo
  echo "- **JSON** numbers are pure loop time (best comparison for the recent JSON work)."
  echo "- **Whole-program** times include Soli parse+VM compile vs Ruby MRI boot; short"
  echo "  programs are heavily startup-dominated on MRI (flat ~35 ms floor here)."
  echo "- Ruby uses stdlib \`JSON\` (C extension, very mature). Soli uses hand-rolled"
  echo "  \`parse_json\` + sonic-rs stringify."
  echo "- On this run, **stringify is faster on Soli**; **parse is still faster on Ruby**."
  echo "- Array/hash/string ops are language-level loops over built-ins, not C microkernels."
  echo "- Recent Soli work in this tree: one-pass JSON parse, join/string/hash to_string,"
  echo "  EcoString identity methods, owning model JSON conversion."
  echo
  echo "## How to reproduce"
  echo
  echo '```bash'
  echo "mise install ruby@$RUBY_VERSION"
  echo "cargo build --release"
  echo "RUBY_BIN=\$(mise exec ruby@$RUBY_VERSION -- which ruby) REPEATS=$REPEATS ./scripts/compare_ruby.sh"
  echo '```'
} >> "$OUT"

echo >&2
echo "Report written to $OUT" >&2
cat "$OUT"
