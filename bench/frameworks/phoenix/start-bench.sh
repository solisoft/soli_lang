#!/usr/bin/env bash
# Phoenix on Bandit, port 5104, MIX_ENV=prod.
#
# There is no `--workers 16` to pass, and that is the point of this column. The
# BEAM is ONE OS process with one scheduler thread per core — 16 on this box, the
# same core budget every other stack gets from its 16 workers — and a lightweight
# process per connection above that. It is the same shape as Soli's 16 worker
# threads in one process; the other five stacks fork 16 OS processes with a heap
# apiece. `+S 16:16` pins the scheduler count so the match is explicit rather
# than inherited from whatever the machine reports.
#
# POOL_SIZE=80 matches Puma's 16x5, Express's and FastAPI's 5-per-worker, and
# Rails' 80 — Ecto holds one pool for the whole VM rather than one per worker.
set -u
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

export MIX_ENV=prod
export PORT="${PORT:-5104}"
export PHX_HOST="${PHX_HOST:-localhost}"
export POOL_SIZE="${POOL_SIZE:-80}"
export DATABASE_URL="${DATABASE_URL:-ecto://bench:bench@127.0.0.1:5433/bench}"
export SECRET_KEY_BASE="${SECRET_KEY_BASE:-$(head -c 48 /dev/urandom | base64 | tr -d '\n')}"
export ERL_AFLAGS="${ERL_AFLAGS:-+S ${SCHEDULERS:-16}:${SCHEDULERS:-16}}"

exec mix phx.server
