#!/usr/bin/env bash
# uvicorn with 16 workers on port 5103, matching every other stack.
#
# `uvicorn --workers` is uvicorn's own supervisor: 16 processes, one
# single-threaded event loop each. With uvloop and httptools installed (the
# `uvicorn[standard]` extra, which is uvicorn's documented production install)
# it picks them up automatically — no flags needed and none given.
#
# Access logging off, matching Django's `--access-logfile /dev/null`.
set -u
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec python3 -m uvicorn \
  --workers "${WORKERS:-16}" \
  --host 127.0.0.1 --port "${PORT:-5103}" \
  --no-access-log --log-level warning \
  benchapp.main:app
