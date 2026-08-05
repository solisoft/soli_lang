#!/usr/bin/env bash
# The labelled reference cells — the "what if you wrote it the other way" rows
# the results page quotes in prose. Same protocol as sweep.sh (8s warm at
# c=100, 30s measured at c=200), so a reference figure is comparable to the
# matched row it sits beside.
#
# Run it right after sweep.sh, in the same session on the same quiet box, or the
# comparison it supports is between two different machines-in-time.
#
#   fastapi /json-encoded  FastAPI's default return path (jsonable_encoder)
#   fastapi /db-encoded    the same, on the DB read
#   fastapi /db-hydrated   50 mapped SQLAlchemy objects instead of a projection
#   django  /db-hydrated   50 Django model objects instead of .values()
#   phoenix /db-hydrated   50 Ecto structs instead of a select map
#   express /db            the raw node-postgres driver, no Sequelize
#   express /db-template   the same, rendered
OUT="${OUT:-/tmp/bench-results-refs}"
mkdir -p "$OUT"

cell() { # stack port path
  local label="$1-$(echo "$3" | tr '/' '-')"
  oha -z 8s  -c 100 --no-tui --output-format quiet "http://localhost:$2$3" >/dev/null 2>&1
  oha -z 30s -c 200 --no-tui --output-format json  "http://localhost:$2$3" > "$OUT/$label.json" 2>/dev/null
  OUT="$OUT" L="$label" python3 -c "
import json, os
d = json.load(open(f\"{os.environ['OUT']}/{os.environ['L']}.json\"))
codes = d['statusCodeDistribution']; n = sum(codes.values())
bad = '' if set(codes) <= {'200','201'} else f'  !! {codes}'
print(f\"  {os.environ['L']:<26} {d['summary']['requestsPerSec']:>9,.0f} req/s  p99 {d['latencyPercentiles']['p99']*1000:>7.2f}ms{bad}\")
"
}

echo "### FastAPI — default return path vs Response"
cell fastapi 5103 /json-encoded
cell fastapi 5103 /db-encoded
cell fastapi 5103 /db-hydrated
echo "### Django — hydrated models vs .values()"
cell django  5099 /db-hydrated
echo "### Phoenix — hydrated structs vs a select map"
cell phoenix 5104 /db-hydrated
echo "### Express — raw driver vs Sequelize"
cell express 5097 /db
cell express 5097 /db-template
echo REFS_DONE
