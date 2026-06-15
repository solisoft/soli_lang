# lib/log.sh — small logging helpers for the grader.
# Two streams:
#   log()  — info, prefixed, to stdout
#   err()  — error, to stderr

if [[ -n "${_SOLI_BENCH_LOG_SH:-}" ]]; then return 0; fi
_SOLI_BENCH_LOG_SH=1

# log LEVEL message...
log() {
  local level="$1"; shift
  printf '  [%s] %s\n' "$level" "$*" >&2
}

err() {
  printf '  [err] %s\n' "$*" >&2
}

die() {
  err "$*"
  exit 1
}
