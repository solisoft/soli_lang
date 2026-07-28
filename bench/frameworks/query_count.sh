#!/usr/bin/env bash
# How many database queries does one request actually issue?
#
# This is the probe that found the `render_json` double-evaluation bug: the
# idiomatic one-liner `render_json(Post.pluck(...).all)` was issuing the query
# twice per request, which read as Soli being slow at database work rather than
# as a bug. Nothing in the response revealed it — only the query log did.
#
#   ./query_count.sh [path-to-soli-binary]
#
# Runs a throwaway app with SOLI_LOG=query against the same SoliDB the suite
# uses, hits each shape three times, and counts the queries. Anything other than
# 3 per shape means a request is issuing more queries than it looks like.
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="${1:-soli}"
# Resolved before any `cd`: the server is started from the app directory, so a
# relative path like ./target/release/soli would no longer point anywhere.
# `command -v` is not the test — it *succeeds* for a relative path that exists,
# which is exactly the case that breaks. Any path with a slash gets resolved.
case "$BIN" in
  */*) BIN="$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")" ;;
esac
PORT="${PORT:-5090}"
APP="${TMPDIR:-/tmp}/soli-query-count-app"
LOG="${TMPDIR:-/tmp}/soli-query-count.log"

rm -rf "$APP"; cp -r "$HERE/soli" "$APP"
cat > "$APP/app/controllers/posts_controller.sl" <<'EOF'
class PostsController < Controller
  # The builder passed inline as a call argument — the shape that used to
  # evaluate twice, and the one the documentation recommends writing.
  def inline(req: Any) -> Any { return render_json(Post.pluck(:id, :title, :views).all) }

  # The same read with the builder bound to a local first.
  def bound(req: Any) -> Any {
    let rows = Post.pluck(:id, :title, :views).all
    return render_json(rows)
  }

  # Through the template engine rather than render_json.
  def templated(req: Any) -> Any {
    return render("posts/list", { "title": "Posts", "items": Post.pluck(:id, :title, :views).all })
  }
end
EOF
cat > "$APP/config/routes.sl" <<'EOF'
get("/inline", "posts#inline")
get("/bound", "posts#bound")
get("/templated", "posts#templated")
EOF

for p in $(pgrep -f "serve . --port $PORT" 2>/dev/null); do kill -9 "$p" 2>/dev/null; done
sleep 1
cd "$APP" || exit 1
SOLI_LOG=query setsid nohup "$BIN" serve . --port "$PORT" --workers 1 > "$LOG" 2>&1 < /dev/null &
disown
for _ in $(seq 1 40); do sleep 1; curl -sf -o /dev/null "http://localhost:$PORT/bound" 2>/dev/null && break; done

echo "queries issued per 3 requests ($BIN) — 3 is correct, 6 means every request queries twice"
status=0
for v in inline bound templated; do
  : > "$LOG"
  for _ in 1 2 3; do curl -s -o /dev/null "http://localhost:$PORT/$v"; done
  sleep 1
  n=$(grep -ao 'FOR doc IN posts' "$LOG" | wc -l)
  flag=""; [ "$n" -ne 3 ] && { flag="  <-- unexpected"; status=1; }
  printf "  %-10s %s%s\n" "$v" "$n" "$flag"
done
for p in $(pgrep -f "serve . --port $PORT" 2>/dev/null); do kill -9 "$p" 2>/dev/null; done
exit $status
