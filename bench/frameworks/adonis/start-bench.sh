#!/usr/bin/env bash
# Start the built AdonisJS app with 16 cluster workers on port 5102.
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PORT="${PORT:-5102}"

for p in $(pgrep -f 'bin/cluster.js' 2>/dev/null); do kill -9 "$p" 2>/dev/null; done
sleep 2

# The build tree is what production runs; it reuses the source node_modules.
ln -sfn "$HERE/node_modules" "$HERE/build/node_modules"
sed -i "s/^PORT=.*/PORT=$PORT/" "$HERE/.env"
cp "$HERE/.env" "$HERE/build/.env"
cp "$HERE/bin/cluster.js" "$HERE/build/bin/cluster.js"

cd "$HERE/build" || exit 1
setsid nohup node bin/cluster.js > /tmp/bench-adonis.log 2>&1 < /dev/null &
disown

for _ in $(seq 1 60); do
  sleep 1
  curl -sf -o /dev/null "http://localhost:$PORT/json" 2>/dev/null && break
done
echo "adonis: $(curl -s -o /dev/null -w '%{http_code}' "http://localhost:$PORT/json") on $PORT"
