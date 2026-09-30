#!/usr/bin/env bash
#
# run_bench.sh — load-test the Soli benchmark app with `oha` and emit reports.
#
# It drives the four workloads the app exposes (json / view / db / crud), one
# `oha` run each, then writes a Markdown + HTML report from the raw JSON.
#
# Usage:
#   ./benchmarks/run_bench.sh                       # defaults below
#   BASE_URL=http://localhost:5011 ./benchmarks/run_bench.sh
#   ./benchmarks/run_bench.sh -z 30s -c 100         # 30s per workload, 100 conns
#
# Prereqs: the app must already be running (e.g. `soli serve .`), plus `oha`,
# `jq` and `curl` on PATH.

set -euo pipefail
export LC_ALL=C   # stable '.' decimal separator across tools

# ---- config (env or flags) ------------------------------------------------
BASE_URL="${BASE_URL:-http://localhost:5011}"
DURATION="${DURATION:-10s}"     # per-workload load duration (oha -z)
CONNECTIONS="${CONNECTIONS:-50}" # concurrent connections (oha -c)
SEED_ROWS="${SEED_ROWS:-1000}"  # rows to ensure in the DB before db/crud runs

while [ $# -gt 0 ]; do
    case "$1" in
        -z|--duration)    DURATION="$2"; shift 2 ;;
        -c|--connections) CONNECTIONS="$2"; shift 2 ;;
        -u|--url)         BASE_URL="$2"; shift 2 ;;
        -h|--help)        sed -n '2,16p' "$0"; exit 0 ;;
        *) echo "unknown arg: $1" >&2; exit 2 ;;
    esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STAMP="$(date +%Y%m%d-%H%M%S)"
OUT_DIR="$SCRIPT_DIR/results/$STAMP"
RAW_DIR="$OUT_DIR/raw"
mkdir -p "$RAW_DIR"

FORM_HDR="Content-Type: application/x-www-form-urlencoded"

# ---- preflight ------------------------------------------------------------
echo "==> Target:      $BASE_URL"
echo "==> Duration:    $DURATION per workload, $CONNECTIONS connections"
echo "==> Output:      $OUT_DIR"

if ! curl -fsS -m 5 "$BASE_URL/health" >/dev/null 2>&1; then
    echo "ERROR: $BASE_URL/health not reachable. Start the app first: soli serve ." >&2
    exit 1
fi

echo "==> Seeding $SEED_ROWS rows ..."
curl -fsS -m 30 "$BASE_URL/bench/seed?n=$SEED_ROWS" >/dev/null

# A concrete row id so the crud read/update workloads hit a real record.
CRUD_ID="$(curl -fsS -m 10 -X POST "$BASE_URL/bench/crud" -d 'name=Seed&price=1' | jq -r '.id')"
echo "==> CRUD target row: $CRUD_ID"

# ---- workloads ------------------------------------------------------------
# Each entry: key|group|label|method|path|body   (body empty for GET)
WORKLOADS=(
    "json|json|Single JSON object|GET|/bench/json|"
    "json_many|json|JSON array (100 items)|GET|/bench/json_many?n=100|"
    "view|view|HTML view (50 rows)|GET|/bench/view?n=50|"
    "page_1|page|Page · step 1 (1 query)|GET|/bench/page?step=1|"
    "page_2|page|Page · step 2 (2 queries)|GET|/bench/page?step=2|"
    "page_3|page|Page · step 3 (3 queries)|GET|/bench/page?step=3|"
    "page_4|page|Page · step 4 (4 queries)|GET|/bench/page?step=4|"
    "db_one|db|DB read: one row|GET|/bench/db_one|"
    "db_list|db|DB read: 100 rows|GET|/bench/db_list?n=100|"
    "crud_create|crud|CRUD create (POST)|POST|/bench/crud|name=Load&price=42"
    "crud_read|crud|CRUD read (GET)|GET|/bench/crud/$CRUD_ID|"
    "crud_update|crud|CRUD update (PUT)|PUT|/bench/crud/$CRUD_ID|price=99"
    "crud_delete|crud|CRUD delete (DELETE)|DELETE|/bench/crud|"
)

run_one() {
    local key="$1" method="$2" path="$3" body="$4"
    local url="$BASE_URL$path"
    local out="$RAW_DIR/$key.json"
    echo "==> [$key] $method $path"
    if [ -z "$body" ]; then
        oha -z "$DURATION" -c "$CONNECTIONS" --no-tui --output-format json \
            -m "$method" "$url" -o "$out"
    else
        oha -z "$DURATION" -c "$CONNECTIONS" --no-tui --output-format json \
            -m "$method" -H "$FORM_HDR" -d "$body" "$url" -o "$out"
    fi
}

for entry in "${WORKLOADS[@]}"; do
    IFS='|' read -r key group label method path body <<< "$entry"
    run_one "$key" "$method" "$path" "$body"
done

# ---- reports --------------------------------------------------------------
echo "==> Building reports ..."
"$SCRIPT_DIR/gen_report.sh" "$OUT_DIR" "$BASE_URL" "$DURATION" "$CONNECTIONS" "$STAMP" <<'WL'
json|json|Single JSON object|GET|/bench/json
json_many|json|JSON array (100 items)|GET|/bench/json_many?n=100
view|view|HTML view (50 rows)|GET|/bench/view?n=50
page_1|page|Page · step 1 (1 query)|GET|/bench/page?step=1
page_2|page|Page · step 2 (2 queries)|GET|/bench/page?step=2
page_3|page|Page · step 3 (3 queries)|GET|/bench/page?step=3
page_4|page|Page · step 4 (4 queries)|GET|/bench/page?step=4
db_one|db|DB read: one row|GET|/bench/db_one
db_list|db|DB read: 100 rows|GET|/bench/db_list?n=100
crud_create|crud|CRUD create (POST)|POST|/bench/crud
crud_read|crud|CRUD read (GET)|GET|/bench/crud/:id
crud_update|crud|CRUD update (PUT)|PUT|/bench/crud/:id
crud_delete|crud|CRUD delete (DELETE)|DELETE|/bench/crud
WL

# Refresh a stable "latest" pointer for convenience.
ln -sfn "$STAMP" "$SCRIPT_DIR/results/latest"

# Push the run into the app DB so the home page (/) shows it as the latest run.
if curl -fsS -m 15 -X POST -H "Content-Type: application/json" \
        --data-binary @"$OUT_DIR/summary.json" "$BASE_URL/bench/results" >/dev/null 2>&1; then
    echo "    Dashboard updated: $BASE_URL/"
else
    echo "    WARNING: could not post results to $BASE_URL/bench/results (dashboard not updated)."
fi

echo ""
echo "==> Done."
echo "    Markdown:  $OUT_DIR/report.md"
echo "    HTML:      $OUT_DIR/report.html"
echo "    Dashboard: $BASE_URL/"
