#!/usr/bin/env bash
# Kemal on port 5105: 16 single-threaded processes sharing the port.
#
# Crystal runs one thread per process unless built with `-Dpreview_mt`, which is
# still a preview; a fleet of processes behind SO_REUSEPORT is how Kemal apps are
# deployed, and it is the shape of the forking stacks (Express's cluster,
# gunicorn, Puma's workers). Each process holds a Jennifer pool of 5, so 16 make
# the 80 connections every stack gets. The kernel spreads incoming connections
# across the 16 listeners.
#
# The binary is built once, in release mode: `shards build --release --production`.
set -u
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

export PORT="${PORT:-5105}"
WORKERS="${WORKERS:-16}"

[ -x bin/bench ] || shards build --release --production

for _ in $(seq 1 "$WORKERS"); do
  ./bin/bench &
done
# Stay in the foreground: start.sh stops a stack by killing the process group of
# whatever listens on its port, which is these 16 and this script.
wait
