#!/bin/bash
S="${SOLI:-./target/release/soli}"
norm() { sed -E 's/ at [0-9]+:[0-9]+//g' | tr '\n' ' '; }
diffs=0; total=0
while IFS= read -r expr; do
  [ -z "$expr" ] && continue
  total=$((total+1))
  w=$($S      -e "print($expr)" 2>&1 | head -2 | norm)
  v=$($S --vm -e "print($expr)" 2>&1 | head -2 | norm)
  if [ "$w" != "$v" ]; then
    diffs=$((diffs+1))
    printf "DIFF  %s\n        walker: %s\n        vm    : %s\n" "$expr" "${w:0:72}" "${v:0:72}"
  fi
done
echo "--- $diffs divergences out of $total ---"
# Exit non-zero so this is usable as a CI gate. Without it the job could only
# ever be green, which is worse than not having the job.
[ "$diffs" -eq 0 ]
