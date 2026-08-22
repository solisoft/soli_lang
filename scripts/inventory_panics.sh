#!/usr/bin/env bash
# Count request-path panics: unwrap / expect / panic! in serve, builtins, vm.
# Hardening Phase 0 — a living inventory, not a gate. Re-run after sweeps.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

paths=(src/serve src/interpreter/builtins src/vm)

echo "== unwrap / expect / panic! / unreachable! by tree =="
printf "%-32s %8s %8s %8s %8s\n" "path" "unwrap" "expect" "panic!" "unreach"
for p in "${paths[@]}"; do
  uw=$(rg -c --pcre2 '\.unwrap\(\)' "$p" -g '*.rs' | awk -F: '{s+=$2} END {print s+0}')
  ex=$(rg -c --pcre2 '\.expect\(' "$p" -g '*.rs' | awk -F: '{s+=$2} END {print s+0}')
  pn=$(rg -c --pcre2 '\bpanic!\(' "$p" -g '*.rs' | awk -F: '{s+=$2} END {print s+0}')
  ur=$(rg -c --pcre2 '\bunreachable!\(' "$p" -g '*.rs' | awk -F: '{s+=$2} END {print s+0}')
  printf "%-32s %8s %8s %8s %8s\n" "$p" "$uw" "$ex" "$pn" "$ur"
done

echo
echo "== .body(..).unwrap() remaining in src/serve (should stay 0 outside tests) =="
rg -n --pcre2 '\.body\([\s\S]{0,200}?\)\s*\.unwrap\(\)' src/serve -g '*.rs' || true

echo
echo "== finish_response call sites =="
rg -n 'finish_response\(' src/serve -g '*.rs' | grep -v 'fn finish_response' || true
