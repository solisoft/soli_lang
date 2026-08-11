#!/usr/bin/env bash
# Run Soli framework-bench smoke + oha on SoliDB, Postgres, and MySQL.
#
# Usage:
#   ./bench-multi-db.sh [out_dir]
#
# Env (set only the backends you have):
#   SOLI_BIN, SOLIDB_HOST, SOLIDB_DATABASE, SOLIDB_USERNAME, SOLIDB_PASSWORD
#   PG_DATABASE_URL, MYSQL_DATABASE_URL
#   WORKERS (default 4), OHA_C (50), OHA_Z (5s), OHA_WARM_Z (1s), PORT_BASE (18080)
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="${1:-./bench-multi-db-out}"
mkdir -p "$OUT"
: >"$OUT/run.log"
: >"$OUT/env-limits.txt"
SOLI_BIN="${SOLI_BIN:-soli}"
WORKERS="${WORKERS:-4}"
OHA_C="${OHA_C:-50}"
OHA_Z="${OHA_Z:-5s}"
OHA_WARM_Z="${OHA_WARM_Z:-1s}"
PORT_BASE="${PORT_BASE:-18080}"

log() { printf '%s\n' "$*" | tee -a "$OUT/run.log"; }

kill_port() {
  local port="$1" pids
  pids=$(lsof -tiTCP:"$port" -sTCP:LISTEN 2>/dev/null || true)
  if [ -n "${pids:-}" ]; then
    # shellcheck disable=SC2086
    kill $pids 2>/dev/null || true
    sleep 0.4
    # shellcheck disable=SC2086
    kill -9 $pids 2>/dev/null || true
  fi
}

wait_ready() {
  local port="$1" i
  for i in $(seq 1 80); do
    if curl -sf -o /dev/null "http://127.0.0.1:${port}/json" 2>/dev/null; then
      return 0
    fi
    sleep 0.25
  done
  return 1
}

smoke_db() {
  local name="$1" port="$2"
  local body_file="$OUT/db-smoke-${name}.json"
  local code
  code=$(curl -sS -o "$body_file" -w '%{http_code}' "http://127.0.0.1:${port}/db" || echo "000")
  echo "$code" >"$OUT/db-smoke-${name}.status"
  if [ "$code" != "200" ]; then
    log "SMOKE FAIL ${name}: HTTP ${code}"
    return 1
  fi
  python3 - "$body_file" <<'PY' || { log "SMOKE FAIL ${name}: body shape"; return 1; }
import json, sys
data = json.load(open(sys.argv[1]))
assert isinstance(data, list) and len(data) > 0, repr(type(data))
row = data[0]
for k in ("id", "title", "views"):
    assert k in row, (k, row)
print("ok", len(data), "rows", "sample", row)
PY
  log "SMOKE OK ${name} /db ($(python3 -c "import json;print(len(json.load(open('$body_file'))))") rows)"
  if curl -sf -o "$OUT/db-template-smoke-${name}.html" "http://127.0.0.1:${port}/db-template"; then
    log "SMOKE OK ${name} /db-template"
  else
    log "SMOKE WARN ${name} /db-template failed"
  fi
}

bench_oha() {
  local name="$1" port="$2" path="$3"
  local slug="${name}${path//\//-}"
  local url="http://127.0.0.1:${port}${path}"
  oha -n 0 -z "$OHA_WARM_Z" -c "$OHA_C" --no-tui "$url" \
    >"$OUT/bench-soli-${slug}-warm.log" 2>&1 || true
  # Prefer JSON output when available (oha --output-format json).
  if oha -n 0 -z "$OHA_Z" -c "$OHA_C" --no-tui --output-format json \
      -o "$OUT/bench-soli-${slug}.json" "$url" \
      >"$OUT/bench-soli-${slug}.log" 2>&1; then
    log "BENCH OK ${name} ${path}"
  else
    oha -n 0 -z "$OHA_Z" -c "$OHA_C" --no-tui "$url" \
      >"$OUT/bench-soli-${slug}.log" 2>&1 || true
    log "BENCH OK ${name} ${path} (text log)"
  fi
}

run_arm() {
  local name="$1" port="$2"
  shift 2
  log "=== arm ${name} on :${port} ==="
  kill_port "$port"

  # Seed under the same adapter env as the server.
  (
    cd "$HERE"
    # Drop any inherited SQL URL so solidb arm does not pick it up.
    unset DATABASE_URL SOLI_DB_ADAPTER || true
    export "$@"
    # Prefer `db:seed` (loads models); fall back to executing seeds.sl.
    if ! "$SOLI_BIN" db:seed . 2>/dev/null; then
      "$SOLI_BIN" db/seeds.sl
    fi
  ) >"$OUT/seed-${name}.log" 2>&1 || log "WARN seed ${name} failed — see seed-${name}.log"

  (
    cd "$HERE"
    unset DATABASE_URL SOLI_DB_ADAPTER || true
    export "$@"
    export SOLI_WS_WORKERS=0
    nohup "$SOLI_BIN" serve . --port "$port" --workers "$WORKERS" \
      >"$OUT/server-${name}.log" 2>&1 &
    echo $! >"$OUT/server-${name}.pid"
  )

  if ! wait_ready "$port"; then
    log "FAIL ${name}: server not ready — see server-${name}.log"
    tail -40 "$OUT/server-${name}.log" | tee -a "$OUT/run.log" || true
    kill_port "$port"
    return 1
  fi

  smoke_db "$name" "$port" || true
  bench_oha "$name" "$port" "/db"
  bench_oha "$name" "$port" "/db-template"
  kill_port "$port"
  return 0
}

summarize() {
  python3 - "$OUT" <<'PY'
import json, sys
from pathlib import Path
out = Path(sys.argv[1])
lines = ["# Soli multi-DB bench summary", "", f"Output dir: `{out}`", ""]
lines += ["| Arm | Path | req/s | p99 (ms) | notes |", "|-----|------|------:|---------:|-------|"]
for p in sorted(out.glob("bench-soli-*.json")):
    try:
        data = json.loads(p.read_text())
    except Exception as e:
        lines.append(f"| {p.name} | ? | — | — | parse error {e} |")
        continue
    rps = data.get("rps") or data.get("requestsPerSec")
    if rps is None and isinstance(data.get("summary"), dict):
        rps = data["summary"].get("requestsPerSec") or data["summary"].get("rps")
    lat = data.get("latencyPercentiles") or data.get("latency") or {}
    p99 = None
    if isinstance(lat, dict):
        p99 = lat.get("p99") or lat.get("99.0") or lat.get("p99Ms")
    # oha sometimes nests under "detail"
    if rps is None and "statusCodeDistribution" in data:
        # try common 1.x JSON
        try:
            rps = data.get("summary", {}).get("requestsPerSec")
        except Exception:
            pass
    stem = p.stem.replace("bench-soli-", "")
    if stem.endswith("-db-template"):
        arm, path = stem[: -len("-db-template")], "/db-template"
    elif stem.endswith("-db"):
        arm, path = stem[: -len("-db")], "/db"
    else:
        arm, path = stem, "?"
    p99ms = p99
    if isinstance(p99, (int, float)) and p99 < 50:
        p99ms = float(p99) * 1000.0  # seconds → ms heuristic
    def f(x):
        if x is None: return "—"
        if isinstance(x, float): return f"{x:.1f}"
        return str(x)
    lines.append(f"| {arm} | {path} | {f(rps)} | {f(p99ms)} | {p.name} |")

lines += ["", "## Smoke status", ""]
for s in sorted(out.glob("db-smoke-*.status")):
    arm = s.stem.replace("db-smoke-", "")
    lines.append(f"- **{arm}**: HTTP {s.read_text().strip()}")
limits = out / "env-limits.txt"
if limits.exists() and limits.read_text().strip():
    lines += ["", "## Env limits", "", "```", limits.read_text().strip(), "```"]
text = "\n".join(lines) + "\n"
(out / "bench-summary.md").write_text(text)
print(text)
PY
}

ran=0
port_i=0

if [ -n "${SOLIDB_HOST:-http://localhost:6745}" ]; then
  # Always try solidb unless SOLIDB_SKIP=1
  if [ "${SOLIDB_SKIP:-0}" != "1" ]; then
    p=$((PORT_BASE + port_i)); port_i=$((port_i + 1))
    args=(
      SOLI_DB_ADAPTER=solidb
      SOLIDB_HOST="${SOLIDB_HOST:-http://localhost:6745}"
      SOLIDB_DATABASE="${SOLIDB_DATABASE:-default}"
    )
    [ -n "${SOLIDB_USERNAME:-}" ] && args+=(SOLIDB_USERNAME="$SOLIDB_USERNAME")
    [ -n "${SOLIDB_PASSWORD:-}" ] && args+=(SOLIDB_PASSWORD="$SOLIDB_PASSWORD")
    [ -n "${SOLIDB_API_KEY:-}" ] && args+=(SOLIDB_API_KEY="$SOLIDB_API_KEY")
    if run_arm solidb "$p" "${args[@]}"; then ran=$((ran + 1)); fi
  else
    echo "solidb: skipped SOLIDB_SKIP=1" >>"$OUT/env-limits.txt"
  fi
fi

if [ -n "${PG_DATABASE_URL:-}" ]; then
  p=$((PORT_BASE + port_i)); port_i=$((port_i + 1))
  if run_arm postgres "$p" \
      SOLI_DB_ADAPTER=postgres \
      DATABASE_URL="$PG_DATABASE_URL"; then
    ran=$((ran + 1))
  fi
else
  echo "postgres: skipped — PG_DATABASE_URL unset" >>"$OUT/env-limits.txt"
  log "SKIP postgres: set PG_DATABASE_URL"
fi

if [ -n "${MYSQL_DATABASE_URL:-}" ]; then
  p=$((PORT_BASE + port_i)); port_i=$((port_i + 1))
  if run_arm mysql "$p" \
      SOLI_DB_ADAPTER=mysql \
      DATABASE_URL="$MYSQL_DATABASE_URL"; then
    ran=$((ran + 1))
  fi
else
  echo "mysql: skipped — MYSQL_DATABASE_URL unset" >>"$OUT/env-limits.txt"
  log "SKIP mysql: set MYSQL_DATABASE_URL"
fi

summarize
if [ "$ran" -eq 0 ]; then
  log "ERROR: no backend completed successfully"
  exit 1
fi
log "Completed ${ran} backend arm(s)"
