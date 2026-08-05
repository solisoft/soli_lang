#!/usr/bin/env bash
# Memory per stack: idle, then at the end of a 30s run on the DB+HTML route.
#
# TWO METHODS, deliberately:
#
#   native stacks  PSS summed over the listener's process group
#   containers     cgroup usage via `docker stats`
#
# Not a style choice. Part of a container's process tree is root-owned, so
# /proc/<pid>/smaps_rollup is unreadable and a PSS sum skips those processes
# *silently* — undercounting, in the flattering direction. That is how Octane
# was once published at 43 MB: the `php artisan octane:start` supervisor alone,
# while the FrankenPHP process holding 230 MB of RSS was skipped. cgroup usage
# and PSS are not the same metric, so the Laravel rows are comparable to each
# other and only indicative against the rest.
set -u
unset NO_COLOR 2>/dev/null || true

pss_pat() {
  local t=0 s
  for p in $(pgrep -f "$1" 2>/dev/null); do
    [ -r "/proc/$p/smaps_rollup" ] && s=$(awk '/^Pss:/{print $2}' "/proc/$p/smaps_rollup" 2>/dev/null) && t=$((t + ${s:-0}))
  done
  echo $((t / 1024))
}

pss_port() {
  local lp pgid procs t=0 s
  lp=$(ss -ltnp 2>/dev/null | grep ":$1 " | grep -oP 'pid=\K[0-9]+' | head -1)
  pgid=$(ps -o pgid= -p "$lp" 2>/dev/null | tr -d ' ')
  procs=$(pgrep -g "$pgid" 2>/dev/null); [ -z "$procs" ] && procs="$lp"
  for p in $procs; do
    [ -r "/proc/$p/smaps_rollup" ] && s=$(awk '/^Pss:/{print $2}' "/proc/$p/smaps_rollup" 2>/dev/null) && t=$((t + ${s:-0}))
  done
  echo $((t / 1024))
}

# Complain rather than undercount: any unreadable process makes a PSS sum wrong.
pss_guard() {
  local unread=0
  for p in $(pgrep -f "$1" 2>/dev/null); do
    [ -r "/proc/$p/smaps_rollup" ] || unread=$((unread + 1))
  done
  [ "$unread" -gt 0 ] && echo "  !! $unread process(es) matching '$1' unreadable — PSS undercounts" >&2
  return 0
}

# Sum MemUsage (MiB) over the named containers.
cmem() {
  local total=0 v
  for c in "$@"; do
    v=$(docker stats --no-stream --format '{{.Name}} {{.MemUsage}}' 2>/dev/null \
        | awk -v n="$c" '$1==n {print $2}' | sed 's/MiB//; s/GiB/*1024/')
    [ -z "$v" ] && v=0
    total=$(python3 -c "print(round($total + ($v), 1))")
  done
  echo "$total"
}

mem() {
  case "$1" in
    laravel) cmem laravel-php-1 laravel-nginx-1 ;;
    octane)  cmem laravel-octane-1 ;;
    django)  pss_guard 'gunicorn.*benchproj'; pss_pat 'gunicorn.*benchproj' ;;
    adonis)  pss_guard 'bin/cluster.js';      pss_pat 'bin/cluster.js' ;;
    *)       pss_port "$2" ;;
  esac
}

printf '%-10s %10s %12s   %s\n' stack idle 'under load' method
# fastapi goes through pss_port, not pss_pat: uvicorn's spawn children have no
# app name in their cmdline (see sweep.sh's srv_cpu note) but do share the
# supervisor's pgid.
for e in soli:5080 rails:5096 express:5097 laravel:5098 octane:5100 django:5099 adonis:5102 fastapi:5103 phoenix:5104; do
  n=${e%%:*}; p=${e##*:}
  case "$n" in laravel | octane) method="cgroup" ;; *) method="PSS" ;; esac
  idle=$(mem "$n" "$p")
  oha -z 30s -c 200 --no-tui --output-format quiet "http://localhost:$p/db-template" >/dev/null 2>&1
  printf '%-10s %7s MB %9s MB   %s\n' "$n" "$idle" "$(mem "$n" "$p")" "$method"
done
echo MEMORY_DONE
