#!/usr/bin/env bash
# Unwrap-count ratchet.
#
# The panic-containment design (`catch_unwind` per request/job worker) only
# works if panics actually unwind — but every `.unwrap()` reachable from user
# Soli code, templates, or network input is a potential process-killer found
# by fuzzing or production traffic. This gate does NOT demand a big-bang fix:
# it freezes today's counts per exposed module and fails when any grows.
# Fixing existing ones lowers the bar permanently.
#
# Usage: scripts/lint_unwraps.sh [--update-baseline]
set -euo pipefail
cd "$(dirname "$0")/.."

BASELINE="scripts/unwrap_baseline.txt"

# Modules whose inputs are untrusted (user code, templates, network).
# Format: path|why it matters
WATCHED="
src/template|template engine renders app-controlled data
src/lexer|lexer sees raw source from users
src/parser|parser sees raw source from users
src/interpreter/value_json.rs|JSON parsing of network bodies
src/interpreter/value_stringify.rs|serialization of runtime values
yaml/src|YAML parser behind Yaml.parse and locale files
src/xml|XML core behind Xml.parse, SOAP.parse and the c14n trio (network bodies)
src/interpreter/builtins/xml/mod.rs|XML binding: registration of Xml, XmlNode, XmlDocument
src/interpreter/builtins/xml/value.rs|XML binding: node handles and the engine hooks
src/interpreter/builtins/xml/node.rs|XML binding: Xml.parse and the node methods (network bodies)
src/interpreter/builtins/xml/block.rs|XML binding: the block builder the engines run
"

count() {
    local path="$1"
    if [ -d "$path" ]; then
        # No match is exit 1, which `pipefail` would turn into an abort.
        { grep -rE '\.(unwrap|expect)\(' "$path" --include='*.rs' || true; } | wc -l | tr -d ' '
    else
        # `grep -c` exits 1 when the count is zero; neutralize under `set -e`.
        { grep -cE '\.(unwrap|expect)\(' "$path" || true; } | tr -d ' '
    fi
}

if [ "${1:-}" = "--update-baseline" ]; then
    printf '%s\n' "$WATCHED" | while IFS='|' read -r path _why; do
        [ -n "$path" ] || continue
        echo "$path $(count "$path")"
    done | sort > "$BASELINE"
    echo "baseline updated:"
    cat "$BASELINE"
    exit 0
fi

if [ ! -f "$BASELINE" ]; then
    echo "missing $BASELINE — run scripts/lint_unwraps.sh --update-baseline" >&2
    exit 2
fi

status=0
while IFS='|' read -r path _why; do
    [ -n "$path" ] || continue
    baseline=$(awk -v p="$path" '$1 == p { print $2 }' "$BASELINE")
    if [ -z "$baseline" ]; then
        baseline=0
    fi
    current=$(count "$path")
    if [ "$current" -gt "$baseline" ]; then
        echo "FAIL $path: $current unwrap/expect calls (baseline $baseline)" >&2
        echo "     $_why" >&2
        echo "     Fix the new call(s), or lower the baseline via --update-baseline if a fix landed." >&2
        status=1
    elif [ "$current" -lt "$baseline" ]; then
        echo "OK   $path: $current < baseline $baseline — run with --update-baseline to lock in the win"
    else
        echo "OK   $path: $current == baseline $baseline"
    fi
done <<EOF
$WATCHED
EOF

exit $status
