#!/bin/bash
# Every shipped example must at least parse.
#
# Three of the five files this first caught used Swift's `\(name)` interpolation
# — a form Soli's lexer rejects outright — and had presumably been failing since
# they were written, because nothing ever ran `examples/`. A reader opening one
# got a lexer error instead of an example.
#
# Parsing, not running: an example may legitimately need a database, a network
# service or a running server. Parsing needs none of that and catches the whole
# class this was written for.
#
#   ./scripts/check-examples.sh
#
# Exits non-zero if any non-skipped example fails to parse.
set -u
SOLI="${SOLI:-./target/release/soli}"

# Files known not to parse, each with a reason. This list should only ever
# shrink — and as of now it is empty. The last two entries were
# `examples/duration.sl`, a 152-line userland reimplementation of the built-in
# `Duration` class written against `static` methods and `x as Float`, and
# `examples/solidb_bench.sl`, written against `new Solidb(...)`. Both were
# rewritten rather than repaired: the constructs they needed are ones Soli has
# never had, and the first was shadowing a builtin that does the same job.
SKIP=()

skipped=0
failed=0
checked=0

for f in $(find examples -name '*.sl' | sort); do
  for s in "${SKIP[@]}"; do
    if [ "$f" = "$s" ]; then
      skipped=$((skipped + 1))
      continue 2
    fi
  done
  checked=$((checked + 1))
  # Only syntax counts. An example may legitimately fail `soli check` on types
  # — a controller helper like `render` is documented as doing exactly that —
  # so type errors are not a failure here, but a Parser or Lexer error is.
  out=$("$SOLI" check "$f" 2>&1)
  if printf '%s' "$out" | grep -q "Unknown option"; then
    echo "the check invocation itself is wrong: $out" >&2
    exit 2
  fi
  err=$(printf '%s' "$out" | grep -E "Parser error|Lexer error" | head -1)
  if [ -n "$err" ]; then
    failed=$((failed + 1))
    printf 'BROKEN  %-40s %s\n' "$f" "$err"
  fi
done

echo "--- $checked example(s) parsed, $failed broken, $skipped skipped ---"
[ "$failed" -eq 0 ]
