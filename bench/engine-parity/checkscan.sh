#!/bin/bash
# Does `soli check` reject a call the runtime accepts? Different axis from
# engine parity: both engines can agree and still be unreachable behind the
# type checker.
S="${SOLI:-./target/release/soli}"
found=0; total=0
while IFS= read -r e; do
  [ -z "$e" ] && continue
  total=$((total+1))
  tc=$($S --vm -e "print($e)" 2>&1 | head -1)
  case "$tc" in *"Type error"*) ;; *) continue ;; esac
  rt=$($S --vm --no-type-check -e "print($e)" 2>&1 | head -1)
  case "$rt" in *Error*) continue ;; esac
  found=$((found+1))
  printf "  %-34s runtime gives: %s\n" "$e" "${rt:0:44}"
done
echo "--- $found checker-rejects-but-runs, out of $total probed ---"
