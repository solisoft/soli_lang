#!/usr/bin/env bash
# Start all seven stacks from a clean slate. Each listens on its own port and
# every one of them gets the same 16-core budget.
#
#   soli 5080 | rails 5096 | express 5097 | laravel 5098 | django 5099
#   fastapi 5103 | phoenix 5104
#
# AdonisJS (5102) needs a build step, so it stays in adonis/start-bench.sh.
# Phoenix is 16 BEAM schedulers in one OS process rather than 16 workers — see
# phoenix/start-bench.sh for why that is the matched configuration.
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export PATH="$HOME/.local/share/gem/ruby/3.4.0/bin:$PATH"

# Only ever kill the benchmark stacks — never a dev fleet sharing this box.
for p in $(pgrep -x soli 2>/dev/null); do
  c=$(tr '\0' ' ' < "/proc/$p/cmdline" 2>/dev/null)
  case "$c" in *"--port 5080"*) kill -9 "$p" 2>/dev/null;; esac
done
# 5103 (uvicorn) is killed by process group, not by cmdline: its workers are
# multiprocessing-spawn children whose cmdline says nothing about the app, so a
# pattern match would leave 16 orphaned workers holding the port.
for port in 5096 5097 5103 5104; do
  lp=$(ss -ltnp 2>/dev/null | grep ":$port " | grep -oP 'pid=\K[0-9]+' | head -1)
  [ -n "$lp" ] && kill -9 -- -"$(ps -o pgid= -p "$lp" | tr -d ' ')" 2>/dev/null
done
for p in $(pgrep -f 'benchproj.wsgi' 2>/dev/null); do kill -9 "$p" 2>/dev/null; done
( cd "$HERE/laravel" && docker compose up -d >/dev/null 2>&1 & )
sleep 3

# SOLI_WS_WORKERS=0 keeps all 16 workers on HTTP; the realtime split would
# otherwise reserve one and the stacks would no longer be matched.
( cd "$HERE/soli"    && SOLI_WS_WORKERS=0 setsid nohup soli serve . --port 5080 --workers 16 \
                          > /tmp/bench-soli.log 2>&1 </dev/null & )
( cd "$HERE/rails"   && RAILS_ENV=production WEB_CONCURRENCY=16 \
                        SECRET_KEY_BASE="${SECRET_KEY_BASE:-$(head -c 64 /dev/urandom | base64 | tr -d '\n')}" \
                        setsid nohup bundle exec puma -C config/puma.rb \
                          > /tmp/bench-rails.log 2>&1 </dev/null & )
( cd "$HERE/express" && setsid nohup node server.js > /tmp/bench-express.log 2>&1 </dev/null & )
( cd "$HERE/django"  && setsid nohup python3 -m gunicorn --workers 16 --bind 127.0.0.1:5099 \
                          --access-logfile /dev/null benchproj.wsgi:application \
                          > /tmp/bench-django.log 2>&1 </dev/null & )
# The flags live in each stack's start-bench.sh so there is one copy of them.
( setsid nohup "$HERE/fastapi/start-bench.sh" > /tmp/bench-fastapi.log 2>&1 </dev/null & )
( setsid nohup "$HERE/phoenix/start-bench.sh" > /tmp/bench-phoenix.log 2>&1 </dev/null & )

for _ in $(seq 1 120); do
  ok=1
  for p in 5080 5096 5097 5098 5099 5103 5104; do
    curl -sf -o /dev/null "http://localhost:$p/db-template" 2>/dev/null || ok=0
  done
  [ "$ok" = 1 ] && break
  sleep 1
done
echo "all seven up: ${ok:-0}"
[ "${ok:-0}" = 1 ] || echo "check /tmp/bench-*.log"
