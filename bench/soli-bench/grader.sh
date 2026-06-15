#!/usr/bin/env bash
# bench/soli-bench/grader.sh — deterministic grader for SoliBench.
#
# Usage:
#   ./grader.sh --reference                              # all suites, all tasks, copy solution.sl
#   ./grader.sh --model <name> --solution-dir <dir>      # use candidate files
#   ./grader.sh --suite core                             # filter
#   ./grader.sh --suite core --task group_by_array       # single task
#   ./grader.sh --json /path/to/results.json             # emit JSON
#
# Exit codes:
#   0  every required task passed
#   1  one or more tasks failed
#   2  bad usage / missing dependency

set -euo pipefail

# ---------------------------------------------------------------------------
# Resolve paths
# ---------------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BENCH_DIR="$SCRIPT_DIR"
LIB_DIR="$BENCH_DIR/lib"
TASKS_JSON="$BENCH_DIR/tasks.json"

# ---------------------------------------------------------------------------
# Source helpers
# ---------------------------------------------------------------------------
# shellcheck source=lib/log.sh
source "$LIB_DIR/log.sh"
# shellcheck source=lib/grade_task.sh
source "$LIB_DIR/grade_task.sh"

# ---------------------------------------------------------------------------
# Defaults
# ---------------------------------------------------------------------------
MODE="reference"
MODEL_NAME="reference"
SOLUTION_DIR=""
SUITE_FILTER=""
TASK_FILTER=""
JSON_OUT=""

# ---------------------------------------------------------------------------
# Arg parsing
# ---------------------------------------------------------------------------
usage() {
  sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'
  exit 2
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --reference)        MODE="reference"; shift ;;
    --model)            MODEL_NAME="$2"; MODE="model"; shift 2 ;;
    --solution-dir)     SOLUTION_DIR="$2"; shift 2 ;;
    --suite)            SUITE_FILTER="$2"; shift 2 ;;
    --task)             TASK_FILTER="$2"; shift 2 ;;
    --json)             JSON_OUT="$2"; shift 2 ;;
    -h|--help)          usage ;;
    *)                  err "unknown arg: $1"; usage ;;
  esac
done

if ! command -v soli >/dev/null 2>&1; then
  err "soli not on PATH. Install with: cargo install --path . --locked"
  exit 2
fi

if ! command -v jq >/dev/null 2>&1; then
  err "jq not on PATH. The grader needs jq to parse tasks.json."
  exit 2
fi

if [[ "$MODE" == "model" && -z "$SOLUTION_DIR" ]]; then
  err "--model requires --solution-dir <dir> mirroring bench/soli-bench/<suite>/<task>/"
  exit 2
fi

# The mvc suite runs against a live SoliDB. Default to an isolated bench
# database so grading never touches an app's data; the host/credentials still
# come from the environment (SOLIDB_HOST / SOLIDB_USERNAME / SOLIDB_PASSWORD).
export SOLIDB_DATABASE="${SOLIDB_DATABASE:-solibench_test}"

# ---------------------------------------------------------------------------
# Discover tasks
# ---------------------------------------------------------------------------
log "Loading manifest: $TASKS_JSON"

# Build the task list (filtered)
read_tasks() {
  jq -c '.tasks[]' "$TASKS_JSON" | while read -r task; do
    local suite name
    suite=$(echo "$task" | jq -r '.suite')
    name=$(echo "$task" | jq -r '.name')
    if [[ -n "$SUITE_FILTER" && "$suite" != "$SUITE_FILTER" ]]; then continue; fi
    if [[ -n "$TASK_FILTER" && "$name" != "$TASK_FILTER" ]]; then continue; fi
    echo "$task"
  done
}

# Collect into an array (bash 3.2 compatible)
TASK_LIST=()
while IFS= read -r line; do
  TASK_LIST+=("$line")
done < <(read_tasks)

if [[ ${#TASK_LIST[@]} -eq 0 ]]; then
  err "No tasks match the filter (suite='${SUITE_FILTER:-*}' task='${TASK_FILTER:-*}')."
  exit 2
fi

log "Found ${#TASK_LIST[@]} task(s) to grade (mode=$MODE, model=$MODEL_NAME)."

# ---------------------------------------------------------------------------
# Grade
# ---------------------------------------------------------------------------
RESULTS_JSON="[]"
PASS=0
FAIL=0
TOTAL_WEIGHT_PASS=0.0
TOTAL_WEIGHT=0.0

# Per-suite tallies
declare -A SUITE_PASS SUITE_TOTAL SUITE_WEIGHT_PASS SUITE_WEIGHT

for task in "${TASK_LIST[@]}"; do
  id=$(echo "$task"    | jq -r '.id')
  suite=$(echo "$task" | jq -r '.suite')
  name=$(echo "$task"  | jq -r '.name')
  weight=$(echo "$task"| jq -r '.weight // 1.0')
  task_dir="$BENCH_DIR/$suite/$name"

  TOTAL_WEIGHT=$(awk "BEGIN{print $TOTAL_WEIGHT + $weight}")
  SUITE_TOTAL[$suite]=$(( ${SUITE_TOTAL[$suite]:-0} + 1 ))
  SUITE_WEIGHT[$suite]=$(awk "BEGIN{print ${SUITE_WEIGHT[$suite]:-0} + $weight}")

  log "[$id] grading..."

  if grade_task "$task_dir" "$MODE" "$MODEL_NAME" "$SOLUTION_DIR"; then
    status="pass"; PASS=$((PASS+1))
    TOTAL_WEIGHT_PASS=$(awk "BEGIN{print $TOTAL_WEIGHT_PASS + $weight}")
    SUITE_PASS[$suite]=$(( ${SUITE_PASS[$suite]:-0} + 1 ))
    SUITE_WEIGHT_PASS[$suite]=$(awk "BEGIN{print ${SUITE_WEIGHT_PASS[$suite]:-0} + $weight}")
  else
    status="fail"; FAIL=$((FAIL+1))
  fi

  RESULTS_JSON=$(echo "$RESULTS_JSON" | jq \
    --arg id "$id" \
    --arg suite "$suite" \
    --arg name "$name" \
    --arg status "$status" \
    --argjson weight "$weight" \
    --argjson details "$(grade_task_last_details)" \
    '. + [{id: $id, suite: $suite, name: $name, status: $status, weight: $weight, details: $details}]')
done

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
SCORE_PCT=$(awk "BEGIN{ if ($TOTAL_WEIGHT > 0) printf \"%.1f\", ($TOTAL_WEIGHT_PASS / $TOTAL_WEIGHT) * 100; else print \"0.0\" }")

log ""
log "=== SoliBench summary ($MODE / $MODEL_NAME) ==="
log "Tasks passed: $PASS / ${#TASK_LIST[@]}"
log "Weighted score: $SCORE_PCT%"

for suite in "${!SUITE_TOTAL[@]}"; do
  s_pass=${SUITE_PASS[$suite]:-0}
  s_total=${SUITE_TOTAL[$suite]}
  s_w=${SUITE_WEIGHT[$suite]}
  s_wp=${SUITE_WEIGHT_PASS[$suite]:-0}
  s_pct=$(awk "BEGIN{ if ($s_w > 0) printf \"%.1f\", ($s_wp / $s_w) * 100; else print \"0.0\" }")
  log "  $suite: $s_pass / $s_total  ($s_pct%)"
done
log ""

# ---------------------------------------------------------------------------
# JSON output
# ---------------------------------------------------------------------------
if [[ -n "$JSON_OUT" ]]; then
  mkdir -p "$(dirname "$JSON_OUT")"
  jq -n \
    --arg mode "$MODE" \
    --arg model "$MODEL_NAME" \
    --argjson passed "$PASS" \
    --argjson total "${#TASK_LIST[@]}" \
    --argjson weight_passed "$TOTAL_WEIGHT_PASS" \
    --argjson weight_total "$TOTAL_WEIGHT" \
    --argjson score_pct "$SCORE_PCT" \
    --argjson results "$RESULTS_JSON" \
    '{
      mode: $mode, model: $model,
      passed: $passed, total: $total,
      weight_passed: $weight_passed, weight_total: $weight_total,
      score_pct: $score_pct,
      results: $results
    }' > "$JSON_OUT"
  log "Wrote JSON: $JSON_OUT"
fi

if [[ $FAIL -gt 0 ]]; then exit 1; fi
exit 0
