#!/usr/bin/env bash
# One publishable session, end to end, with the box checked on both sides.
#
#   control -> sweep -> refs -> restart -> memory -> control
#
# The trailing control is the point. A pre-flight check cannot catch a box that
# degrades *mid-run*, and that is exactly what spoiled two earlier attempts: a
# sweep that started at load 0.50 and finished with every stack halved. If the
# closing control fails, the session is discarded rather than published — the
# numbers are not wrong in any way a reader could detect, which is what makes
# them dangerous.
#
# Waits for a quiet box first, and retries the opening control rather than
# giving up, so this can be left running (or cron'd) on a shared machine.
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$HERE"

OUT="${OUT:-/tmp/bench-session-$(date +%H%M%S 2>/dev/null || echo run)}"
STACKS="${STACKS:-soli rails express laravel django adonis fastapi phoenix octane}"
MAX_TRIES="${MAX_TRIES:-40}"
LOAD_GATE="${LOAD_GATE:-2.5}"

say() { echo "[$(date +%H:%M:%S)] $*"; }

wait_quiet() {
  local waited=0
  while [ "$(awk -v g="$LOAD_GATE" '{print ($1<g)?1:0}' /proc/loadavg)" != 1 ]; do
    sleep 30; waited=$((waited+30))
    [ $((waited % 300)) = 0 ] && say "still waiting on load ($(cut -d' ' -f1-3 /proc/loadavg))"
  done
}

say "target dir: $OUT"
for try in $(seq 1 "$MAX_TRIES"); do
  say "attempt $try/$MAX_TRIES — waiting for load < $LOAD_GATE"
  wait_quiet
  say "load $(cut -d' ' -f1-3 /proc/loadavg) — opening control"
  if ./control.sh; then break; fi
  say "control failed; the box is not comparable. sleeping 10m"
  [ "$try" = "$MAX_TRIES" ] && { say "giving up after $MAX_TRIES attempts"; exit 1; }
  sleep 600
done

say "=== sweep ==="
OUT="$OUT" STACKS="$STACKS" ./sweep.sh || { say "sweep failed"; exit 1; }

say "=== reference cells ==="
wait_quiet
OUT="$OUT-refs" ./refs.sh || { say "refs failed"; exit 1; }

say "=== closing control — did the box hold? ==="
wait_quiet
if ! ./control.sh; then
  say "!! THE BOX DRIFTED DURING THE RUN. Discard $OUT — do not publish it."
  exit 2
fi

# Memory last, and only after a restart: read after an hour of sweeping, these
# same servers report retained heap rather than an idle footprint.
say "=== restarting stacks for a true idle memory reading ==="
./start.sh >/dev/null 2>&1
./adonis/start-bench.sh >/dev/null 2>&1
sleep 10
wait_quiet
say "=== memory ==="
./memory.sh || { say "memory failed"; exit 1; }

say "SESSION_OK — $OUT is publishable (both controls passed)"
