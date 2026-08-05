#!/bin/bash
# Every cell of the comparison, all seven stacks, one sweep, back to back.
#
# No load-average gate between cells: a 30s run at c=200 inflates the 1-minute
# average itself, so gating on it just makes every cell wait out its own
# predecessor. Check the box is quiet once before starting; the per-cell 8s
# warm-up is the settle.
OUT="${OUT:-/tmp/bench-results}"
mkdir -p "$OUT"
PGURL="${PGURL:-postgres://bench:bench@127.0.0.1:5433/bench}"
SDB="${SDB:-http://localhost:6745/_api/database/default}"

listener() { ss -ltnp 2>/dev/null | grep ":$1 " | grep -oP 'pid=\K[0-9]+' | head -1; }
cpu_grp() {
  local pgid procs t=0 u s
  pgid=$(ps -o pgid= -p "$1" 2>/dev/null | tr -d ' ')
  procs=$(pgrep -g "$pgid" 2>/dev/null); [ -z "$procs" ] && procs="$1"
  for p in $procs; do read -r _ _ _ _ _ _ _ _ _ _ _ _ _ u s _ < /proc/$p/stat 2>/dev/null && t=$((t+u+s)); done
  echo "$t"
}
cpu_pat() {  # sum over every process matching a pattern (Laravel: fpm + nginx)
  local t=0 u s
  for p in $(pgrep -f "$1" 2>/dev/null); do read -r _ _ _ _ _ _ _ _ _ _ _ _ _ u s _ < /proc/$p/stat 2>/dev/null && t=$((t+u+s)); done
  echo "$t"
}
cpu_one() { local u s; read -r _ _ _ _ _ _ _ _ _ _ _ _ _ u s _ < /proc/$1/stat 2>/dev/null || { echo 0; return; }; echo $((u+s)); }
DBPID=$(listener 6745)

# phoenix also stays on the default branch, for the opposite reason to fastapi:
# the BEAM is a single OS process (16 scheduler threads inside it), and
# /proc/<pid>/stat already aggregates every thread's CPU — same shape as Soli.
#
# fastapi deliberately has no cpu_pat branch. uvicorn's 16 workers are
# multiprocessing-spawn children whose cmdline is `python3 -c from
# multiprocessing.spawn import spawn_main...` — nothing about the app — so
# `cpu_pat 'benchapp.main'` matches the supervisor alone and reported 8 ticks
# where the real process group had 88. They do share the supervisor's pgid, so
# the default cpu_grp branch counts all 17. Leave it there.
# octane needs a pattern for the opposite reason to fastapi: its processes live in
# a container, so `ss -ltnp` cannot see the listener's pid as a normal user and
# cpu_grp silently summed nothing — the column read 0us for every Octane cell.
# /proc/<pid>/stat is world-readable even for root-owned processes (unlike
# smaps_rollup, which is why memory.sh has to use cgroups here instead), so a
# pattern works. Both processes count: the octane:start supervisor and the
# frankenphp worker host.
srv_cpu() { case "$1" in laravel) cpu_pat 'php-fpm|nginx';; django) cpu_pat 'gunicorn.*benchproj';; adonis) cpu_pat 'bin/cluster.js';; octane) cpu_pat 'octane:start|frankenphp run';; *) cpu_grp "$(listener "$2")";; esac; }

pg_count()  { psql "$PGURL" -tAc "SELECT count(*) FROM wposts;"; }
sdb_count() { curl -s -u admin:admin -X POST $SDB/cursor -H 'Content-Type: application/json' \
                -d '{"query":"RETURN COUNT(FOR d IN wposts RETURN 1)"}' \
              | python3 -c "import json,sys; print(json.load(sys.stdin)['result'][0])"; }
reset_pg()  { psql "$PGURL" -qc "TRUNCATE wposts;" \
    -c "INSERT INTO wposts (id,title,views) SELECT g,'Post title '||g,g*7 FROM generate_series(1,800000) g;" \
    -c "SELECT setval(pg_get_serial_sequence('wposts','id'),900000);" >/dev/null; }
reset_sdb() {
  curl -s -u admin:admin -X DELETE $SDB/collection/wposts >/dev/null
  curl -s -u admin:admin -X POST $SDB/collection -H 'Content-Type: application/json' -d '{"name":"wposts"}' >/dev/null
  for b in 0 1 2 3 4 5 6 7; do lo=$((b*100000+1)); hi=$(((b+1)*100000))
    curl -s -u admin:admin -X POST $SDB/cursor -H 'Content-Type: application/json' \
      -d "{\"query\":\"FOR i IN $lo..$hi INSERT { _key: TO_STRING(i), title: CONCAT(\\\"Post title \\\", i), views: i * 7 } INTO wposts RETURN 1\"}" -o /dev/null
  done
}

cell() { # stack port method path [write]
  local stack=$1 port=$2 method=$3 path=$4 write=$5 label c0 c1 d0 d1 before after
  label="$stack$(echo "$path" | tr '/' '-')-$method"
  if [ -n "$write" ]; then
    case "$stack" in soli) reset_sdb; before=$(sdb_count);; *) reset_pg; before=$(pg_count);; esac
  fi
  oha -z 8s -c 100 -m "$method" --no-tui --output-format quiet "http://localhost:$port$path" >/dev/null 2>&1
  [ -n "$write" ] && { case "$stack" in soli) reset_sdb;; *) reset_pg;; esac; }
  c0=$(srv_cpu "$stack" "$port"); d0=$(cpu_one "$DBPID")
  oha -z 30s -c 200 -m "$method" --no-tui --output-format json "http://localhost:$port$path" > "$OUT/$label.json" 2>/dev/null
  c1=$(srv_cpu "$stack" "$port"); d1=$(cpu_one "$DBPID")
  if [ -n "$write" ]; then case "$stack" in soli) after=$(sdb_count);; *) after=$(pg_count);; esac; else before=0; after=0; fi
  OUT="$OUT" L="$label" S="$stack" SRV=$((c1-c0)) DBC=$((d1-d0)) BEFORE="${before:-0}" AFTER="${after:-0}" W="$write" python3 -c "
import json, os
d = json.load(open(f\"{os.environ['OUT']}/{os.environ['L']}.json\"))
codes = d['statusCodeDistribution']; n = sum(codes.values())
srv = int(os.environ['SRV'])/100; dbc = int(os.environ['DBC'])/100
extra = f'  (+solidb {dbc/n*1e6:.0f}={(srv+dbc)/n*1e6:.0f}us sys)' if os.environ['S']=='soli' and dbc>2 else ''
rows = ''
if os.environ['W']:
    delta = int(os.environ['AFTER'])-int(os.environ['BEFORE'])
    rows = f'  rows {abs(delta)/n*100:.0f}%'
bad = '' if set(codes) <= {'200','201'} else f'  !! {codes}'
print(f\"  {os.environ['S']:<8} {d['summary']['requestsPerSec']:>9,.0f} req/s  p99 {d['latencyPercentiles']['p99']*1000:>7.2f}ms  CPU/req {srv/n*1e6:>6.0f}us{extra}{rows}{bad}\")
"
}

# Octane is a reference configuration, not a stack in its own right, so it is
# off by default: STACKS="soli rails express laravel django adonis octane" adds it.
STACKS="${STACKS:-soli rails express laravel django adonis fastapi phoenix}"

declare -A PORT=([soli]=5080 [rails]=5096 [express]=5097 [laravel]=5098 [django]=5099 [adonis]=5102 [octane]=5100 [fastapi]=5103 [phoenix]=5104)
# Express serves the ORM form of the DB routes; the others use their only form.
url_for() { case "$1:$2" in express:/db) echo /db-orm;; express:/db-template) echo /db-template-orm;; *) echo "$2";; esac; }

for wl in /json /template /db /db-template; do
  echo "### $wl"
  for s in $STACKS; do cell "$s" "${PORT[$s]}" GET "$(url_for $s $wl)"; done
done
for m in POST PATCH DELETE; do
  echo "### $m /w"
  for s in $STACKS; do cell "$s" "${PORT[$s]}" "$m" /w w; done
done
echo SWEEP_DONE
