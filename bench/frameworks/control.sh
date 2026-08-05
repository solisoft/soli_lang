#!/usr/bin/env bash
# Is this box in the same state as the session that produced the published
# numbers? Run this BEFORE a sweep, not after.
#
# Two cells whose published values are known, on the two stacks least likely to
# be the thing that changed. If either is off by more than TOLERANCE percent,
# the box is not comparable and a sweep would publish degraded numbers dressed
# as a regression — which is how a benchmark page loses its credibility.
#
# This exists because it already happened: a sweep started at load 0.50 read
# Express at 89,008 against a published 129,725 (-31%) and Rails at 12,072
# against 16,116 (-25%), because other tenants on the box had woken up. Nothing
# in the harness noticed; the numbers just looked like a regression.
set -u
unset NO_COLOR 2>/dev/null || true
TOLERANCE="${TOLERANCE:-8}"

# stack:port:path:expected
# Baselines from the last published session on this page (2026-08). Override with
# CONTROLS=... when re-baselining after a stack or protocol change.
CONTROLS="${CONTROLS:-express:5097:/json:109263 soli:5080:/template:124933}"

fail=0
printf '%-10s %-14s %10s %10s %8s\n' stack route measured published drift
for c in $CONTROLS; do
  IFS=: read -r stack port path expected <<< "$c"

  # Reachability first. `oha` counts connection failures as completed requests
  # and reports a rate for them, so a stack that is simply not running comes back
  # as a spectacular number: a dead port measured 256,510 req/s against a
  # published 127,287 and read as "+101% — box got faster". This check exists
  # because the control tool had the exact bug it is meant to catch.
  code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 "http://localhost:$port$path" 2>/dev/null)
  if [ "$code" != "200" ]; then
    printf '%-10s %-14s %10s %10s %8s   <-- NOT SERVING (HTTP %s)\n' \
      "$stack" "$path" "-" "$expected" "-" "${code:-none}"
    fail=1
    continue
  fi

  oha -z 8s  -c 100 --no-tui --output-format quiet "http://localhost:$port$path" >/dev/null 2>&1
  json=$(oha -z 20s -c 200 --no-tui --output-format json "http://localhost:$port$path" 2>/dev/null)
  # Reject the cell unless every response was a success, for the same reason.
  read -r got okcodes <<< "$(echo "$json" | python3 -c "
import json, sys
d = json.load(sys.stdin)
codes = d['statusCodeDistribution']
print(int(d['summary']['requestsPerSec']), 1 if set(codes) <= {'200','201'} else 0)
")"
  if [ "$okcodes" != "1" ]; then
    printf '%-10s %-14s %10s %10s %8s   <-- NON-2xx RESPONSES\n' \
      "$stack" "$path" "$got" "$expected" "-"
    fail=1
    continue
  fi

  drift=$(python3 -c "print(f'{($got-$expected)/$expected*100:+.1f}%')")
  bad=$(python3 -c "print(1 if abs($got-$expected)/$expected*100 > $TOLERANCE else 0)")
  [ "$bad" = 1 ] && fail=1
  printf '%-10s %-14s %10s %10s %8s %s\n' "$stack" "$path" "$got" "$expected" "$drift" \
    "$([ "$bad" = 1 ] && echo '  <-- OUT OF TOLERANCE')"
done

echo
if [ "$fail" = 1 ]; then
  echo "NOT COMPARABLE — do not sweep. Check: load average, other tenants"
  echo "(ps -eo pcpu,comm --sort=-pcpu | head), swap (free -h), and whether a"
  echo "stack needs restarting to shed a grown heap."
  exit 1
fi
echo "COMPARABLE — safe to sweep."
