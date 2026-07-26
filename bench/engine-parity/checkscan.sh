#!/bin/bash
# Does `soli check` reject a call the runtime accepts?
#
# A different axis from engine parity: both engines can agree perfectly and
# still leave a method unreachable, because a declared arity narrower than the
# implementation turns a working call into a check-time error.
#
# A rejected-but-running call falls into one of two buckets, and only one is a
# checker bug:
#
#   CHECKER  the runtime *honours* the extra argument — removing it changes the
#            answer — so the call really works and the declaration is too
#            narrow. This is the actionable one.
#
#   LAX      the runtime *ignores* the extra argument — the answer is identical
#            without it. Here the checker is right to reject, and the laxity is
#            in the runtime, which should raise a wrong-arity error rather than
#            silently discard input. Lower severity: type-checked code, which is
#            the default, never reaches it.
#
# They are told apart by re-running with the last argument removed and comparing.
# Reported separately so the actionable list stays short and keeps meaning
# something.
#
#   ./bench/engine-parity/checkscan.sh < bench/engine-parity/arity-probes.txt
#
# Exits non-zero when any CHECKER-class mismatch is found.
S="${SOLI:-./target/release/soli}"
checker=0
lax=0
total=0
lax_list=""

while IFS= read -r expr; do
  [ -z "$expr" ] && continue
  total=$((total + 1))

  # Only interested in calls the checker turns away...
  tc=$("$S" --vm -e "print($expr)" 2>&1 | head -1)
  case "$tc" in *"Type error"*) ;; *) continue ;; esac

  # ...but not on a type mismatch. The corpus is generated with stand-in
  # arguments, so `{"a": 1}.has_value?("x")` is rejected for passing a String
  # where the hash's value type is Int — a correct rejection, and nothing to do
  # with the declared arity this script is hunting for.
  case "$tc" in *"Type mismatch"*) continue ;; esac

  # ...that the runtime nonetheless runs.
  rt=$("$S" --vm --no-type-check -e "print($expr)" 2>&1 | head -1)
  case "$rt" in *Error*) continue ;; esac

  # Drop the final argument and see whether the answer moves. If it does not,
  # the argument was never used.
  stripped=$(printf '%s' "$expr" | sed -E 's/, *[^,()]+\)$/)/; t; s/\([^()]+\)$/()/')
  if [ "$stripped" != "$expr" ]; then
    base=$("$S" --vm --no-type-check -e "print($stripped)" 2>&1 | head -1)
    if [ "$base" = "$rt" ]; then
      lax=$((lax + 1))
      lax_list="$lax_list  $expr\n"
      continue
    fi
  fi

  checker=$((checker + 1))
  printf "CHECKER  %-34s runtime gives: %s\n" "$expr" "${rt:0:44}"
done

echo "--- $checker checker-too-narrow (actionable), $lax runtime-lax, out of $total probed ---"
if [ "$lax" -gt 0 ]; then
  echo "runtime-lax (checker is right; the runtime silently ignores the extra argument):"
  printf "$lax_list" | sort -u | head -40
fi
[ "$checker" -eq 0 ]
