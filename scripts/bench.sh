#!/usr/bin/env bash
# scripts/bench.sh — reproducible HTTP throughput bench for a Soli app.
#
# Builds the release binary if needed, starts a `soli serve` worker detached,
# runs an oha warmup + measurement pass, then cleans up. Results are written
# to stdout and to $BENCH_OUT (default: /tmp/soli_bench_result.txt).
#
# Usage:
#   ./scripts/bench.sh                          # bench /tmp/soli_bench (default)
#   ./scripts/bench.sh path/to/app              # bench a specific app
#   PORT=5020 WORKERS=8 CONNECTIONS=200 DURATION=20s ./scripts/bench.sh
#   LABEL=baseline ./scripts/bench.sh
#
# Env vars (all optional):
#   APP             path to a Soli app dir (default: /tmp/soli_bench)
#   PORT            port to bind           (default: 5011)
#   WORKERS         worker threads         (default: nproc)
#   CONNECTIONS     oha -c                 (default: 400)
#   WARMUP          warmup duration        (default: 5s)
#   DURATION        measurement duration   (default: 15s)
#   LABEL           short tag for the run  (default: <timestamp>)
#   SKIP_BUILD=1    don't rebuild soli
#   KEEP_SERVER=1   leave soli running on exit
#   OUT_DIR         where to write logs    (default: /tmp/soli_bench_logs)
#
# Requires: oha, cargo, curl, ss (iproute2). Run on the target machine.

set -euo pipefail

APP="${APP:-/tmp/soli_bench}"
PORT="${PORT:-5011}"
WORKERS="${WORKERS:-$(nproc)}"
CONNECTIONS="${CONNECTIONS:-400}"
WARMUP="${WARMUP:-5s}"
DURATION="${DURATION:-15s}"
LABEL="${LABEL:-$(date +%Y%m%d-%H%M%S)}"
OUT_DIR="${OUT_DIR:-/tmp/soli_bench_logs}"
SKIP_BUILD="${SKIP_BUILD:-0}"
KEEP_SERVER="${KEEP_SERVER:-0}"

SOLI_BIN="${SOLI_BIN:-/home/olivier.bonnaure@delupay.com/.cargo/bin/soli}"
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

mkdir -p "$OUT_DIR"
SERVER_LOG="$OUT_DIR/${LABEL}_server.log"
RESULT_FILE="$OUT_DIR/${LABEL}_result.txt"

log()  { printf '[bench] %s\n' "$*" >&2; }
fail() { printf '[bench] ERROR: %s\n' "$*" >&2; exit 1; }

command -v oha >/dev/null  || fail "oha not found in PATH"
command -v curl >/dev/null || fail "curl not found in PATH"
command -v ss >/dev/null   || fail "ss not found in PATH (install iproute2)"

if [[ ! -d "$APP" ]]; then
  log "App $APP not found; scaffolding a fresh soli new app there"
  mkdir -p "$(dirname "$APP")"
  ( cd "$(dirname "$APP")" && "$SOLI_BIN" new "$(basename "$APP")" >/dev/null 2>&1 ) \
    || fail "soli new failed; ensure $SOLI_BIN is built and you have write access"
fi

# Build release binary if stale or missing.
LOCAL_BIN="$REPO_ROOT/target/release/soli"
need_build=1
if [[ "$SKIP_BUILD" == "1" ]]; then need_build=0; fi
if [[ "$need_build" == "1" && -x "$LOCAL_BIN" && "$LOCAL_BIN" -nt "$REPO_ROOT/Cargo.toml" ]]; then
  need_build=0
fi
if [[ "$need_build" == "1" ]]; then
  log "Building release binary (cargo build --release) ..."
  ( cd "$REPO_ROOT" && cargo build --release ) > "$OUT_DIR/${LABEL}_build.log" 2>&1 \
    || fail "cargo build failed; see $OUT_DIR/${LABEL}_build.log"
fi
BIN_TO_USE="${SOLI_OVERRIDE:-$LOCAL_BIN}"
[[ -x "$BIN_TO_USE" ]] || BIN_TO_USE="$SOLI_BIN"
log "Using soli binary: $BIN_TO_USE"
log "App:               $APP"
log "Port:              $PORT"
log "Workers:           $WORKERS"
log "Connections:       $CONNECTIONS"
log "Warmup / Duration: $WARMUP / $DURATION"
log "Output:            $RESULT_FILE + $SERVER_LOG"

# Free the port if something is on it.
if ss -ltn "sport = :$PORT" 2>/dev/null | tail -n +2 | grep -q LISTEN; then
  log "Port $PORT already in use; killing the listener"
  PIDS=$(ss -ltnp "sport = :$PORT" 2>/dev/null | tail -n +2 | grep -oP 'pid=\K[0-9]+' | sort -u)
  for p in $PIDS; do kill "$p" 2>/dev/null || true; done
  sleep 1
fi

# Start server detached.
setsid nohup "$BIN_TO_USE" serve "$APP" --port "$PORT" --workers "$WORKERS" \
  > "$SERVER_LOG" 2>&1 < /dev/null &
SERVER_PID=$!
disown "$SERVER_PID" 2>/dev/null || true

cleanup() {
  if [[ "${KEEP_SERVER:-0}" == "1" ]]; then
    log "KEEP_SERVER=1; leaving server (pid=$SERVER_PID) on port $PORT"
    return
  fi
  if kill -0 "$SERVER_PID" 2>/dev/null; then
    kill "$SERVER_PID" 2>/dev/null || true
    sleep 0.5
    kill -9 "$SERVER_PID" 2>/dev/null || true
  fi
  pkill -P "$SERVER_PID" 2>/dev/null || true
}
trap cleanup EXIT

# Wait for the listener to be ready.
for i in $(seq 1 50); do
  if ss -ltn "sport = :$PORT" 2>/dev/null | tail -n +2 | grep -q LISTEN; then
    break
  fi
  sleep 0.2
done
ss -ltn "sport = :$PORT" 2>/dev/null | tail -n +2 | grep -q LISTEN \
  || { tail -50 "$SERVER_LOG" >&2; fail "server did not start within 10s"; }

# Warmup (JIT, mimalloc, page cache).
log "Warmup: oha -c $CONNECTIONS -z $WARMUP ..."
oha -c "$CONNECTIONS" -z "$WARMUP" --no-tui "http://localhost:$PORT/" >/dev/null 2>&1 || true

# RSS sample before measurement.
RSS_KB=$(ps -o rss= -p "$SERVER_PID" 2>/dev/null | tr -d ' ' || echo 0)

# Measurement.
log "Measuring: oha -c $CONNECTIONS -z $DURATION ..."
oha -c "$CONNECTIONS" -z "$DURATION" --no-tui "http://localhost:$PORT/" \
  | tee "$RESULT_FILE"

# Spot-check the OOP bench endpoint vs a function-style one if present.
if curl -sf -o /dev/null "http://localhost:$PORT/health"; then
  log "Health check: OK"
fi

# Summary
SUMMARY="$OUT_DIR/${LABEL}_summary.txt"
{
  echo "label:       $LABEL"
  echo "binary:      $BIN_TO_USE"
  echo "app:         $APP"
  echo "port:        $PORT"
  echo "workers:     $WORKERS"
  echo "connections: $CONNECTIONS"
  echo "duration:    $DURATION"
  echo "rss_kb:      $RSS_KB"
  echo "soli_ver:    $($BIN_TO_USE --version 2>&1 | head -1)"
  echo "result:      $RESULT_FILE"
  echo "server_log:  $SERVER_LOG"
} | tee "$SUMMARY"

log "Done. Summary: $SUMMARY"
