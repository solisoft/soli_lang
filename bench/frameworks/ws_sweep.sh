#!/usr/bin/env bash
# WebSocket cells for the stacks that serve them.
#
# The load generator is SHARDED across processes, and that is not optional: a
# single Node client saturates long before the servers do. Measured against
# Soli, one client process reported 74k msg/s, two 139k, four 230k, eight 238k —
# the first number was the harness, not the server. Anything measured with one
# client process is a floor.
#
# Only Soli and Express are here. Rails needs ActionCable on Redis, Django needs
# Channels on ASGI and Laravel needs Reverb — each is a different server process
# from the one serving their HTTP rows, so adding them is not a handler change.
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export NODE_PATH="$HERE/express/node_modules"
CONNS="${CONNS:-2000}"
SECS="${SECS:-10}"
SHARDS="${SHARDS:-8}"
PER=$(( CONNS / SHARDS ))

pss_port() {
  local lp pgid procs t=0 s
  lp=$(ss -ltnp 2>/dev/null | grep ":$1 " | grep -oP 'pid=\K[0-9]+' | head -1)
  pgid=$(ps -o pgid= -p "$lp" 2>/dev/null | tr -d ' '); procs=$(pgrep -g "$pgid" 2>/dev/null); [ -z "$procs" ] && procs="$lp"
  for p in $procs; do
    [ -r "/proc/$p/smaps_rollup" ] && s=$(awk '/^Pss:/{print $2}' "/proc/$p/smaps_rollup" 2>/dev/null) && t=$((t+${s:-0}))
  done
  echo $((t/1024))
}

# Run `mode` on `port` across $SHARDS client processes; sum the given key.
sharded() { # mode port path key
  local mode=$1 port=$2 path=$3 key=$4 i
  for i in $(seq 1 "$SHARDS"); do
    node "$HERE/ws_bench.js" "$mode" "ws://localhost:$port$path" "$PER" "$SECS" \
      > "/tmp/ws-shard-$i.json" 2>/dev/null &
  done
  wait
  KEY="$key" SHARDS="$SHARDS" python3 -c "
import json, os, glob
tot, p50, p99, conns, extra = 0, [], [], 0, []
for f in sorted(glob.glob('/tmp/ws-shard-*.json')):
    try: d = json.load(open(f))
    except Exception: continue
    tot += d.get(os.environ['KEY'], 0); conns += d.get('connections', d.get('opened', 0))
    if 'p50_ms' in d: p50.append(d['p50_ms']); p99.append(d['p99_ms'])
    if 'fanout_per_publish' in d: extra.append(d['fanout_per_publish'])
out = {'total': tot, 'conns': conns}
if p50: out['p50'] = sum(p50)/len(p50); out['p99'] = max(p99)
if extra: out['fanout'] = sum(extra)/len(extra)
print(json.dumps(out))
"
  rm -f /tmp/ws-shard-*.json
}

# CAVEAT: the memory delta in this cell is only trustworthy from a fresh
# `./start.sh`. It samples the listener's process group, which on a box that has
# already run sweeps can pick up unrelated workers and report nonsense (a 3.4 GB
# baseline, or a negative per-connection cost). Restart before believing it; the
# connection counts and connect rate are fine either way.
echo "### capacity — $CONNS concurrent connections held open"
for e in soli:5080 express:5097; do
  n=${e%%:*}; p=${e##*:}
  base=$(pss_port "$p")
  node "$HERE/ws_bench.js" capacity "ws://localhost:$p/ws/echo" "$CONNS" > /tmp/ws-cap.json 2>/dev/null &
  bench=$!; sleep 3; held=$(pss_port "$p"); wait $bench
  N=$n BASE=$base HELD=$held python3 -c "
import json, os
d = json.load(open('/tmp/ws-cap.json'))
base, held = int(os.environ['BASE']), int(os.environ['HELD'])
per = (held-base)*1024.0/max(d['opened'],1)
print(f\"  {os.environ['N']:<8} {d['opened']:>6,} open ({d['failed']} failed)  {d['connects_per_sec']:>6,}/s connect  mem {base}->{held} MB ({per:.0f} KB/conn)\")
"
done

echo "### echo — round trip, one message in flight per connection ($SHARDS client processes)"
for e in soli:5080 express:5097; do
  n=${e%%:*}; p=${e##*:}
  sharded echo "$p" /ws/echo msgs_per_sec | N=$n python3 -c "
import json, os, sys
d = json.load(sys.stdin)
print(f\"  {os.environ['N']:<8} {d['total']:>9,} msg/s  p50 {d.get('p50',0):>7.3f}ms  p99 {d.get('p99',0):>7.3f}ms  ({d['conns']:,} conns)\")
"
done

# Room is deliberately NOT sharded: one room, one publisher. Sharding would
# create eight publishers and split the room between them, which measures
# something else. The publisher is rate-limited (PUBLISH_RATE, default 50/s) so
# fan-out per publish stays meaningful instead of measuring the client's send
# loop. The number to read is fan-out/publish against the connection count — if
# it is lower, the broadcast is not reaching the whole room.
echo "### room — one publisher, rate-limited; every connection should receive"
for e in soli:5080 express:5097; do
  n=${e%%:*}; p=${e##*:}
  node "$HERE/ws_bench.js" room "ws://localhost:$p/ws/room" "$CONNS" "$SECS" 2>/dev/null | N=$n python3 -c "
import json, os, sys
d = json.load(sys.stdin)
share = d['fanout_per_publish'] / max(d['connections'], 1) * 100
print(f\"  {os.environ['N']:<8} fan-out {d['fanout_per_publish']:>7.1f}/publish of {d['connections']:,} conns ({share:.0f}% of the room)  {d['deliveries_per_sec']:>8,} deliveries/s\")
"
done
echo WS_SWEEP_DONE
