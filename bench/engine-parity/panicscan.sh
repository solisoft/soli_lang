#!/bin/bash
# Can hostile input reach a Rust panic through a Soli method?
#
# A panic is not just an ugly error. Release builds unwind now, and a
# per-request guard turns a panicking handler into a 500 — but a panic still
# aborts the work in progress and is never the intended way to report bad input.
# Anything reachable from user code should come back as a Soli error instead.
#
#   ./bench/engine-parity/panicscan.sh < bench/engine-parity/hostile-inputs.txt
#
# Both engines are run, because they have separate dispatchers and separate
# bounds handling. A clean Soli error exits 70 and is fine; only "panicked at"
# and an abort-shaped exit code count as a finding.
#
# Exits non-zero if anything panics.
S="${SOLI:-./target/release/soli}"
found=0
total=0
while IFS= read -r expr; do
  [ -z "$expr" ] && continue
  total=$((total + 1))
  for eng in "" "--vm"; do
    out=$("$S" $eng --no-type-check -e "print($expr)" 2>&1)
    rc=$?
    if printf '%s' "$out" | grep -q "panicked at" || [ "$rc" -gt 100 ]; then
      found=$((found + 1))
      printf "PANIC%-6s %-36s rc=%-4s %s\n" "$eng" "$expr" "$rc" \
        "$(printf '%s' "$out" | tr '\n' ' ' | cut -c1-58)"
    fi
  done
done
echo "--- $found panics across $total expressions x2 engines ---"
[ "$found" -eq 0 ]
