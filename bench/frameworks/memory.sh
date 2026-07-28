#!/usr/bin/env bash
# PSS per stack: idle, then at the end of a 30s run on the DB+HTML route.
pss_pat() { local t=0 s; for p in $(pgrep -f "$1" 2>/dev/null); do
    [ -r "/proc/$p/smaps_rollup" ] && s=$(awk '/^Pss:/{print $2}' "/proc/$p/smaps_rollup" 2>/dev/null) && t=$((t+${s:-0})); done; echo $((t/1024)); }
pss_port() { local lp pgid procs t=0 s
  lp=$(ss -ltnp 2>/dev/null | grep ":$1 " | grep -oP 'pid=\K[0-9]+' | head -1)
  pgid=$(ps -o pgid= -p "$lp" 2>/dev/null | tr -d ' '); procs=$(pgrep -g "$pgid" 2>/dev/null); [ -z "$procs" ] && procs="$lp"
  for p in $procs; do [ -r "/proc/$p/smaps_rollup" ] && s=$(awk '/^Pss:/{print $2}' "/proc/$p/smaps_rollup" 2>/dev/null) && t=$((t+${s:-0})); done; echo $((t/1024)); }
mem() { case "$1" in laravel) pss_pat 'php-fpm|nginx';; django) pss_pat 'gunicorn.*benchproj';; *) pss_port "$2";; esac; }

printf '%-10s %10s %12s\n' stack idle 'under load'
for e in soli:5080 rails:5096 express:5097 laravel:5098 django:5099; do
  n=${e%%:*}; p=${e##*:}
  idle=$(mem "$n" "$p")
  oha -z 30s -c 200 --no-tui --output-format quiet "http://localhost:$p/db-template" >/dev/null 2>&1
  printf '%-10s %7s Mo %9s Mo\n' "$n" "$idle" "$(mem "$n" "$p")"
done
echo MEMORY_DONE
