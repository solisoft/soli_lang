#!/usr/bin/env bash
# A/B the SoliDB transport: HTTP (reqwest) against the native driver protocol.
#
# Same binary, same app, same box, alternating configurations — so the only
# variable is `SOLI_DB_DRIVER`. Requires a build with `--features solidb-driver`
# (the feature is off by default; see Cargo.toml).
#
# Cells are interleaved A,B per workload rather than run as two blocks, so a
# slow drift in machine load hits both arms instead of only the second.
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="${BIN:-$HERE/../../target/release/soli}"
PORT=5080
WORKERS="${WORKERS:-16}"
WARM="${WARM:-8s}"
DUR="${DUR:-20s}"
CONC="${CONC:-200}"
REPS="${REPS:-2}"
SDB="${SDB:-http://localhost:6745/_api/database/default}"

reset_sdb() {
  curl -s -u admin:admin -X DELETE "$SDB/collection/wposts" >/dev/null
  curl -s -u admin:admin -X POST "$SDB/collection" -H 'Content-Type: application/json' \
       -d '{"name":"wposts"}' >/dev/null
  for b in 0 1 2 3 4 5 6 7; do
    lo=$((b*100000+1)); hi=$(((b+1)*100000))
    curl -s -u admin:admin -X POST "$SDB/cursor" -H 'Content-Type: application/json' \
      -d "{\"query\":\"FOR i IN $lo..$hi INSERT { _key: TO_STRING(i), title: CONCAT(\\\"Post title \\\", i), views: i * 7 } INTO wposts RETURN 1\"}" -o /dev/null
  done
}

stop() {
  local lp
  lp=$(ss -ltnp 2>/dev/null | grep ":$PORT " | grep -oP 'pid=\K[0-9]+' | head -1)
  [ -n "$lp" ] && kill -9 -- -"$(ps -o pgid= -p "$lp" | tr -d ' ')" 2>/dev/null
  sleep 2
}

start() { # $1 = "http" | "driver"
  stop
  local env_drv=""
  [ "$1" = "driver" ] && env_drv="1"
  ( cd "$HERE/soli" && SOLI_DB_DRIVER="$env_drv" SOLI_WS_WORKERS=0 \
      setsid nohup "$BIN" serve . --port $PORT --workers "$WORKERS" \
      > "/tmp/soli-ab-$1.log" 2>&1 </dev/null & )
  for _ in $(seq 1 60); do
    sleep 1
    curl -sf -o /dev/null "http://localhost:$PORT/db" 2>/dev/null && break
  done
  # Refuse to measure a driver arm that silently fell back to HTTP.
  if [ "$1" = "driver" ] && grep -q "falling back to HTTP" "/tmp/soli-ab-$1.log" 2>/dev/null; then
    echo "  !! driver arm fell back to HTTP — aborting"; exit 1
  fi
}

cpu_of_port() {
  local lp pgid t=0 u s
  lp=$(ss -ltnp 2>/dev/null | grep ":$PORT " | grep -oP 'pid=\K[0-9]+' | head -1)
  pgid=$(ps -o pgid= -p "$lp" 2>/dev/null | tr -d ' ')
  for p in $(pgrep -g "${pgid:-0}" 2>/dev/null); do
    read -r _ _ _ _ _ _ _ _ _ _ _ _ _ u s _ < /proc/$p/stat 2>/dev/null && t=$((t+u+s))
  done
  echo "$t"
}
db_cpu() {
  local lp t=0 u s
  lp=$(ss -ltnp 2>/dev/null | grep ':6745 ' | grep -oP 'pid=\K[0-9]+' | head -1)
  read -r _ _ _ _ _ _ _ _ _ _ _ _ _ u s _ < /proc/${lp:-0}/stat 2>/dev/null && t=$((u+s))
  echo "$t"
}

cell() { # $1 arm, $2 method, $3 path, $4 is_write
  [ -n "$4" ] && reset_sdb
  oha -z "$WARM" -c 100 -m "$2" --no-tui --output-format quiet "http://localhost:$PORT$3" >/dev/null 2>&1
  [ -n "$4" ] && reset_sdb
  local c0 d0 c1 d1
  c0=$(cpu_of_port); d0=$(db_cpu)
  oha -z "$DUR" -c "$CONC" -m "$2" --no-tui --output-format json "http://localhost:$PORT$3" > /tmp/ab.json 2>/dev/null
  c1=$(cpu_of_port); d1=$(db_cpu)
  ARM="$1" SRV=$((c1-c0)) DBC=$((d1-d0)) python3 -c "
import json, os
d = json.load(open('/tmp/ab.json'))
codes = d['statusCodeDistribution']; n = sum(codes.values())
srv = int(os.environ['SRV'])/100; dbc = int(os.environ['DBC'])/100
bad = '' if set(codes) <= {'200','201'} else f'  !! {codes}'
print(f\"    {os.environ['ARM']:<7} {d['summary']['requestsPerSec']:>9,.0f} req/s  \"
      f\"p99 {d['latencyPercentiles']['p99']*1000:>7.2f}ms  \"
      f\"soli {srv/n*1e6:>5.0f}us  +solidb {dbc/n*1e6:>5.0f}us  \"
      f\"= {(srv+dbc)/n*1e6:>5.0f}us sys{bad}\")
"
}

printf '%s\n' "transport A/B — $WORKERS workers, c=$CONC, ${DUR} measured after ${WARM} warm, ${REPS} reps/arm"
printf '%s\n' "load at start: $(cut -d' ' -f1-3 /proc/loadavg)"
for wl in ${WL:-"GET:/db:" "GET:/db-template:" "POST:/w:w" "PATCH:/w:w" "DELETE:/w:w"}; do
  IFS=: read -r method path is_write <<< "$wl"
  echo "### $method $path"
  for arm in http driver; do
    start "$arm"
    for _ in $(seq 1 "$REPS"); do
      cell "$arm" "$method" "$path" "$is_write"
    done
  done
done
stop
echo AB_DONE
