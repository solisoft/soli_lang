# lib/grade_task.sh — grade a single SoliBench task.
#
# Public entry point:
#   grade_task <task_dir> <mode:reference|model> <model_name> <solution_dir>
#
#   Sets global:
#     GRADE_LAST_DETAILS  — JSON string with the last task's diagnostic
#
#   Returns:
#     0 on pass, 1 on fail.

if [[ -n "${_SOLI_BENCH_GRADE_TASK_SH:-}" ]]; then return 0; fi
_SOLI_BENCH_GRADE_TASK_SH=1

# Source log helpers if not already loaded.
if [[ -z "${_SOLI_BENCH_LOG_SH:-}" ]]; then
  # shellcheck source=log.sh
  source "$(dirname "${BASH_SOURCE[0]}")/log.sh"
fi

# ---------------------------------------------------------------------------
# grade_task_last_details — echo JSON of the last task's diagnostic.
# ---------------------------------------------------------------------------
grade_task_last_details() {
  printf '%s' "${GRADE_LAST_DETAILS:-null}"
}

# ---------------------------------------------------------------------------
# run_test_and_lint — run `soli test` + `soli lint` in a temp dir.
#
#   Sets:
#     TEST_STDERR, TEST_STATUS
#     LINT_STDERR, LINT_STATUS
#     LINT_JSON
#   Args:
#     $1 = scratch dir
#     $2 = "with-lint" to also run lint, "no-lint" otherwise
# ---------------------------------------------------------------------------
run_test_and_lint() {
  local dir="$1" mode="$2" lint_target="${3:-tests/}"

  TEST_STATUS=0
  TEST_STDERR=$( (cd "$dir" && timeout 60 soli test tests/ --no-coverage 2>&1) ) || TEST_STATUS=$?
  LINT_STATUS=0
  LINT_STDERR=""
  LINT_JSON="[]"
  if [[ "$mode" == "with-lint" ]]; then
    LINT_STDERR=$( (cd "$dir" && timeout 30 soli lint "$lint_target" 2>&1) ) || LINT_STATUS=$?
  fi
}

# ---------------------------------------------------------------------------
# grade_task <task_dir> <mode> <model_name> <solution_dir>
# ---------------------------------------------------------------------------
grade_task() {
  local task_dir="$1" mode="$2" model_name="$3" solution_dir="$4"
  GRADE_LAST_DETAILS='null'

  local prompt="$task_dir/prompt.md"
  local solution="$task_dir/solution.sl"
  local stub="$task_dir/stub.sl"
  local tests="$task_dir/tests.sl"
  local meta="$task_dir/meta.json"

  if [[ ! -f "$prompt" || ! -f "$tests" || ! -f "$stub" ]]; then
    err "task $task_dir missing prompt.md/tests.sl/stub.sl"
    return 1
  fi

  local suite
  suite=$(basename "$(dirname "$task_dir")")

  # Pick the candidate file:
  #   reference mode: copy solution.sl
  #   model mode:     <solution_dir>/<suite>/<task>.sl (or .sl.tmpl)
  local scratch
  scratch=$(mktemp -d -t solibench.XXXXXX)
  trap 'rm -rf "$scratch"' RETURN

  # The Soli test runner requires:
  #   - a `tests/` directory containing the spec
  #   - a `.env.test` file in app_dir (the parent of `tests/`)
  #   - a `tests/<name>.sl` file (any .sl suffix works; discovery is recursive)
  mkdir -p "$scratch/tests"
  touch "$scratch/.env.test"

  if [[ "$mode" == "reference" ]]; then
    [[ -f "$solution" ]] || { err "$task_dir: missing solution.sl for reference mode"; return 1; }
    cp "$solution" "$scratch/solution.sl"
  else
    local candidate
    for cand in "$solution_dir/$suite/$(basename "$task_dir").sl" \
                "$solution_dir/$suite/$(basename "$task_dir")/solution.sl" \
                "$solution_dir/$(basename "$task_dir").sl"; do
      if [[ -f "$cand" ]]; then candidate="$cand"; break; fi
    done
    if [[ -z "${candidate:-}" ]]; then
      err "$task_dir: no candidate under $solution_dir (looked at $suite/$(basename "$task_dir").sl etc.)"
      GRADE_LAST_DETAILS=$(jq -nc '{error: "missing candidate"}')
      return 1
    fi
    cp "$candidate" "$scratch/solution.sl"
  fi

  # Build the spec file: include the solution first, then the test body.
  local spec_name
  spec_name="$(basename "$task_dir")_spec.sl"
  {
    printf '# Auto-generated runner for SoliBench task: %s\n' "$(basename "$task_dir")"
    printf '# Suite: %s\n' "$suite"
    printf '\n# --- solution.sl ---\n'
    cat "$scratch/solution.sl"
    printf '\n# --- tests.sl ---\n'
    cat "$tests"
  } > "$scratch/tests/$spec_name"

  # Use 'with-lint' for the idiom suite, 'no-lint' elsewhere.
  #
  # For idiom tasks we lint the *candidate solution alone* (not the
  # solution+tests spec), placed under `app/controllers/` so that
  # path-sensitive rules (e.g. style/redundant-model-import) fire just as they
  # would in a real project. Test helpers in tests.sl must not count against
  # the lint score, and the controller location is what makes the model-import
  # rule observable.
  local lint_mode="no-lint"
  local lint_target="tests/"
  if [[ "$suite" == "idiom" ]]; then
    lint_mode="with-lint"
    mkdir -p "$scratch/app/controllers"
    cp "$scratch/solution.sl" "$scratch/app/controllers/$(basename "$task_dir").sl"
    lint_target="app/controllers/$(basename "$task_dir").sl"
  fi

  run_test_and_lint "$scratch" "$lint_mode" "$lint_target"

  local pass=1
  if [[ $TEST_STATUS -ne 0 ]]; then pass=0; fi
  if [[ "$lint_mode" == "with-lint" && $LINT_STATUS -ne 0 ]]; then pass=0; fi

  # Build details JSON.
  local details
  details=$(jq -nc \
    --arg test_stdout "$TEST_STDERR" \
    --argjson test_status "$TEST_STATUS" \
    --arg lint_stdout "$LINT_STDERR" \
    --argjson lint_status "$LINT_STATUS" \
    --argjson lint_json "$LINT_JSON" \
    '{
      test: { status: $test_status, output: $test_stdout },
      lint: { status: $lint_status, output: $lint_stdout, json: $lint_json }
    }')
  GRADE_LAST_DETAILS="$details"

  if [[ $pass -eq 1 ]]; then
    log "ok"
    return 0
  else
    err "FAILED (test=$TEST_STATUS lint=$LINT_STATUS)"
    if [[ -n "$TEST_STDERR" ]]; then
      printf '%s\n' "$TEST_STDERR" | sed 's/^/    | /' >&2
    fi
    if [[ "$lint_mode" == "with-lint" && -n "$LINT_STDERR" ]]; then
      printf '%s\n' "$LINT_STDERR" | sed 's/^/    | /' >&2
    fi
    return 1
  fi
}
