#!/usr/bin/env python3
"""Builtin API coverage audit.

Checks which Soli builtins (globals, classes, methods) are exercised by the
.sli-level test suite under tests/.

Inventory sources (authoritative):
  1. Runtime environment dump via `cargo run --example builtin_inventory`
     -> globals + registered classes and their native/static methods.
  2. src/interpreter/executor/calls/method_registry.rs
     -> primitive-type method tables (Int/Float/Decimal/Bool/Null/Symbol/
        String/Array/Hash/QueryBuilder), which live in code, not in Class maps.

Usage:
  python3 scripts/builtin_audit.py                # writes builtin_coverage_report.md
  python3 scripts/builtin_audit.py --json out.json  # also emit machine-readable gaps
"""

import argparse
import json
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REGISTRY = ROOT / "src/interpreter/executor/calls/method_registry.rs"
TESTS_DIR = ROOT / "tests"
DEFAULT_REPORT = ROOT / "builtin_coverage_report.md"

# Alias groups: a method is "used" if ANY member of its group appears.
ALIASES = [
    {"len", "length", "size"},
    {"to_s", "to_string"},
    {"to_a", "to_array"},
    {"to_i", "to_int"},
    {"to_f", "to_float"},
    {"succ", "next"},
    {"push", "append", "add"},
    {"has_key", "key?", "include?"},
]

TABLE_LABELS = {
    "INT_METHODS": "Int",
    "FLOAT_METHODS": "Float",
    "DECIMAL_METHODS": "Decimal",
    "BOOL_METHODS": "Bool",
    "NULL_METHODS": "Null",
    "SYMBOL_METHODS": "Symbol",
    "STRING_METHODS": "String*",
    "ARRAY_METHODS": "Array*",
    "HASH_METHODS": "Hash*",
    "QUERY_BUILDER_METHODS": "QueryBuilder",
}

def strip_comment(line: str) -> str:
    """Drop a trailing `#`/`//` comment without touching string literals.

    A blunt `re.sub(r"#.*|//.*", "", line)` truncated at the first `#`, which in
    Soli is also the start of `#{...}` interpolation — so every call inside an
    interpolation (`"import-#{tenant.upcase()}"`) vanished from the scan and was
    reported as an uncovered gap. It also ate anything after the `//` in a URL
    literal.
    """
    out = []
    quote = None
    i = 0
    while i < len(line):
        ch = line[i]
        if quote:
            out.append(ch)
            if ch == "\\" and i + 1 < len(line):
                out.append(line[i + 1])
                i += 2
                continue
            if ch == quote:
                quote = None
            i += 1
            continue
        if ch in "\"'":
            quote = ch
            out.append(ch)
            i += 1
            continue
        # `#{` is interpolation, not a comment.
        if ch == "#" and not line.startswith("#{", i):
            break
        if ch == "/" and line.startswith("//", i):
            break
        out.append(ch)
        i += 1
    return "".join(out)


def source_stamp() -> str:
    """Identity of the source the inventory was built from.

    HEAD sha plus the newest mtime under `src/`, so an edited or newly added
    builtin invalidates the cache. Without this the audit happily reported on a
    previous run's surface: new methods never showed up as gaps and the totals
    were wrong, with nothing on stderr to say the numbers were stale.
    """
    try:
        head = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            capture_output=True, text=True, cwd=ROOT, check=True,
        ).stdout.strip()
    except (subprocess.CalledProcessError, OSError):
        head = "no-git"
    newest = max(
        (p.stat().st_mtime_ns for p in (ROOT / "src").rglob("*.rs")),
        default=0,
    )
    return f"{head}:{newest}"


def build_inventory(cache: Path) -> dict:
    stamp = source_stamp()
    if cache.exists():
        try:
            cached = json.loads(cache.read_text())
        except json.JSONDecodeError:
            cached = None
        if isinstance(cached, dict) and cached.get("_source_stamp") == stamp:
            return cached
        if cached is not None:
            print("Inventory cache is stale (src/ changed); rebuilding ...", file=sys.stderr)
    print("Running `cargo run --example builtin_inventory` ...", file=sys.stderr)
    result = subprocess.run(
        ["cargo", "run", "--example", "builtin_inventory"],
        capture_output=True,
        text=True,
        cwd=ROOT,
        check=True,
    )
    # Cargo prints compile progress to stderr; stdout is pure JSON.
    inv = json.loads(result.stdout)
    inv["_source_stamp"] = stamp
    cache.write_text(json.dumps(inv))
    return inv


def parse_method_registry() -> dict[str, list[str]]:
    """Extract per-type method tables from method_registry.rs."""
    src = REGISTRY.read_text()
    tables: dict[str, list[str]] = {}
    current = None
    for line in src.splitlines():
        m = re.match(r"pub const ([A-Z_]+_METHODS)", line)
        if m:
            current = m.group(1)
            tables[current] = []
            continue
        if current:
            for name in re.findall(r'name:\s*"([^"]+)"', line):
                tables[current].append(name)
    return {TABLE_LABELS.get(k, k): v for k, v in tables.items()}


def load_test_sources(include_rust_embedded: bool) -> list[str]:
    lines: list[str] = []
    for path in sorted(TESTS_DIR.rglob("*.sl")):
        for line in path.read_text(errors="replace").splitlines():
            stripped = strip_comment(line)
            lines.append(stripped)
    if include_rust_embedded:
        # Rust integration tests embed Soli snippets; scan them too (noisy but
        # errs toward counting coverage rather than reporting false gaps).
        for path in sorted(TESTS_DIR.rglob("*.rs")):
            lines.append(path.read_text(errors="replace"))
    return lines


RECEIVER_CALL_RE = re.compile(r"\.\s*([A-Za-z_][A-Za-z0-9_?!]*)")
# `Toml.stringify(...)` — a class-qualified call, so a static method can be
# attributed to the right class instead of any same-named method anywhere.
QUALIFIED_CALL_RE = re.compile(r"\b([A-Z][A-Za-z0-9_]*)\s*\.\s*([A-Za-z_][A-Za-z0-9_?!]*)")
GLOBAL_CALL_RE = re.compile(r"(?<![.\w])([A-Za-z_][A-Za-z0-9_?!]*)\s*\(")
SYMBOL_PROC_RE = re.compile(r"&\s*:\s*([A-Za-z_][A-Za-z0-9_?!]*)")


def collect_used_names(lines: list[str]) -> tuple[set[str], set[str], set[tuple[str, str]]]:
    used_methods: set[str] = set()
    used_globals: set[str] = set()
    text = "\n".join(lines)
    used_methods.update(RECEIVER_CALL_RE.findall(text))
    used_methods.update(SYMBOL_PROC_RE.findall(text))
    used_globals.update(GLOBAL_CALL_RE.findall(text))
    qualified = set(QUALIFIED_CALL_RE.findall(text))
    return used_methods, used_globals, qualified


def alias_expand(names: set[str], method: str) -> bool:
    for group in ALIASES:
        if method in group and group & names:
            return True
    return False


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--json", metavar="PATH", help="also write machine-readable gap list")
    ap.add_argument(
        "--include-rust-embedded",
        action="store_true",
        help="also scan Soli snippets embedded in tests/**/*.rs",
    )
    ap.add_argument("--report", metavar="PATH", default=str(DEFAULT_REPORT))
    ap.add_argument(
        "--inventory-cache",
        default="/tmp/builtin_inventory.json",
        help="reuse this inventory JSON if present; delete to force a rebuild",
    )
    args = ap.parse_args()

    inv = build_inventory(Path(args.inventory_cache))
    registry_tables = parse_method_registry()
    lines = load_test_sources(args.include_rust_embedded)
    used_methods, used_globals, used_qualified = collect_used_names(lines)
    # Bare-name globals (the unqualified part of every registered global).
    global_bare_names = {
        (g["name"].split(".", 1)[1] if "." in g["name"] else g["name"])
        for g in inv["globals"]
    }

    def is_used(method: str) -> bool:
        # A method counts as referenced if its name appears receiver-style
        # (`.method(`), as symbol-to-proc, OR as a bare call — model DSL
        # members (`has_many(...)`) are invoked unqualified inside class
        # bodies, and instance-first variants of global functions are invoked
        # receiver-style.
        return (
            method in used_methods
            or method in used_globals
            or alias_expand(used_methods, method)
            or alias_expand(used_globals, method)
        )

    def is_used_static(cls: str, method: str) -> bool:
        """A static method counts when called on ITS OWN class, or via its
        global DSL shadow.

        Many class statics are *also* registered as bare globals so they can
        be called unqualified inside class bodies (`has_many("posts")` in a
        model definition routes through the global `has_many` to the enclosing
        class). For those, bare usage of the global credits the static — that
        IS the documented calling convention. Statics with no global shadow
        (e.g. `Toml.stringify`) still require a properly qualified call, so
        `Yaml.stringify` cannot cover `Toml.stringify`. Instance methods keep
        the bare-name heuristic: a receiver's type is not knowable from text.
        """
        if (cls, method) in used_qualified or any(
            (cls, alias) in used_qualified
            for group in ALIASES
            if method in group
            for alias in group
        ):
            return True
        return method in global_bare_names and (
            method in used_globals
            or any(alias in used_globals for group in ALIASES if method in group for alias in group)
        )

    report: list[str] = []
    gaps: dict = {}

    total_items = 0
    covered_items = 0
    class_rows = []
    class_gaps: dict[str, list[str]] = {}

    # --- classes & their methods ---
    for cls in inv["classes"]:
        statics = set(cls["static_methods"])
        methods = sorted(set(cls["instance_methods"]) | statics)
        if not methods:
            continue
        uncovered = [
            m
            for m in methods
            if not (
                is_used_static(cls["name"], m)
                if m in statics and m not in set(cls["instance_methods"])
                else is_used(m)
            )
        ]
        covered = len(methods) - len(uncovered)
        total_items += len(methods)
        covered_items += covered
        pct = 100 * covered / len(methods)
        class_rows.append((cls["name"], len(methods), covered, pct))
        if uncovered:
            class_gaps[cls["name"]] = uncovered

    # --- primitive-type methods from the central registry ---
    prim_rows = []
    prim_gaps: dict[str, list[str]] = {}
    for table, methods in registry_tables.items():
        uniq = sorted(set(methods))
        uncovered = [m for m in uniq if not is_used(m)]
        total_items += len(uniq)
        covered_items += len(uniq) - len(uncovered)
        pct = 100 * (len(uniq) - len(uncovered)) / len(uniq)
        prim_rows.append((table, len(uniq), len(uniq) - len(uncovered), pct))
        if uncovered:
            prim_gaps[table] = uncovered

    # --- globals ---
    # Globals are credited by bare calls AND receiver-style calls (Solidb
    # instances expose `inst.solidb_get(...)`, `Model.validates(...)` etc.).
    all_global_names = sorted({g["name"] for g in inv["globals"]})
    # __-prefixed globals are internal plumbing (template/enum/mailer internals),
    # not public API — reported separately, not counted against coverage.
    internal_globals = [g for g in all_global_names if g.startswith("__")]
    global_names = [g for g in all_global_names if not g.startswith("__")]
    # Namespaced globals like "Factory.bind" are invoked as Factory.bind(...):
    # the qualified regex catches `.bind(`, so credit via the bare part too.
    global_uncovered = [
        g for g in global_names
        if not (
            g in used_globals
            or g in used_methods
            or ("." in g and g.rsplit(".", 1)[1] in used_methods)
        )
    ]
    total_items += len(global_names)
    covered_items += len(global_names) - len(global_uncovered)

    overall_pct = 100 * covered_items / total_items if total_items else 0.0

    # Second pass: the schema/state-machine DSL is invoked Ruby-style without
    # parentheses (`columnar compression: "lz4"`, `state_machine :state do`),
    # which call-shaped patterns cannot see. Any item still unresolved gets one
    # last word-boundary check over the raw source before being reported.
    def mentioned(name: str) -> bool:
        return re.search(rf"\b{re.escape(name)}\b", "\n".join(lines)) is not None

    for cls in list(class_gaps):
        class_gaps[cls] = [m for m in class_gaps[cls] if not mentioned(m)]
        if not class_gaps[cls]:
            del class_gaps[cls]
    for table in list(prim_gaps):
        prim_gaps[table] = [m for m in prim_gaps[table] if not mentioned(m)]
        if not prim_gaps[table]:
            del prim_gaps[table]
    global_uncovered = [g for g in global_uncovered if not mentioned(g.rsplit(".", 1)[-1])]

    # Recompute tallies from the post-second-pass gap lists so the report's
    # numbers and its gap lists always agree.
    class_rows = [
        (name, total, total - len(class_gaps.get(name, [])),
         100 * (total - len(class_gaps.get(name, []))) / total)
        for name, total, _, _ in class_rows
    ]
    prim_rows = [
        (name, total, total - len(prim_gaps.get(name, [])),
         100 * (total - len(prim_gaps.get(name, []))) / total)
        for name, total, _, _ in prim_rows
    ]
    covered_items = sum(cov for _, _, cov, _ in class_rows + prim_rows) + (
        len(global_names) - len(global_uncovered)
    )
    overall_pct = 100 * covered_items / total_items if total_items else 0.0

    report.append("# Builtin API Coverage Report")
    report.append("")
    report.append(
        "> NOT AUDITED: methods that exist only as a match arm in "
        "`src/interpreter/executor/access/member.rs` — the universal members "
        "(`class`, `nil?`, `blank?`, `present?`, `inspect`, `to_s`, `is_a?`) and "
        "their per-type variants, including DateTime's. They are in neither the "
        "runtime inventory nor the registry tables, so they are absent from the "
        "totals below rather than counted as covered."
    )
    report.append("")
    report.append(f"- API entries audited: **{total_items}**")
    report.append(f"- Referenced by tests/: **{covered_items}** ({overall_pct:.1f}%)")
    report.append(f"- Unreferenced: **{total_items - covered_items}**")
    report.append("")
    report.append("> Heuristic scan of `tests/**/*.sl` for `.method` / `global(` usage.")
    report.append("> Dot-access without parens counts (auto-invoked methods), which")
    report.append("> over-counts coverage where property-style hash access coincides with")
    report.append("> a method name. Treat single-method gaps as advisory.")
    report.append("")

    def section(rows, gaps_map, title):
        report.append(f"## {title}")
        report.append("")
        report.append("| Surface | Methods | Covered | % |")
        report.append("|---|---|---|---|")
        for name, total, cov, pct in rows:
            flag = "" if pct == 100 else " ⚠️" if pct >= 50 else " ❌"
            report.append(f"| {name} | {total} | {cov} | {pct:.0f}%{flag} |")
        report.append("")
        for name, uncovered in sorted(gaps_map.items()):
            report.append(f"### Uncovered: {name}")
            report.append("")
            report.append("`" + "` `".join(uncovered) + "`")
            report.append("")

    section(class_rows, class_gaps, "Classes")
    section(prim_rows, prim_gaps, "Primitive / universal methods (method_registry.rs)")

    report.append("## Global functions")
    report.append("")
    report.append(f"{len(global_names) - len(global_uncovered)} / {len(global_names)} referenced")
    report.append("")
    if global_uncovered:
        report.append("### Uncovered globals")
        report.append("")
        for g in global_uncovered:
            report.append(f"- `{g}`")
        report.append("")
    if internal_globals:
        report.append(f"### Excluded internal globals ({len(internal_globals)})")
        report.append("")
        report.append("`" + "` `".join(internal_globals) + "`")
        report.append("")

    Path(args.report).write_text("\n".join(report))
    print(f"Wrote {args.report}")
    print(f"Overall: {covered_items}/{total_items} ({overall_pct:.1f}%) API entries referenced")

    if args.json:
        payload = {
            "overall_pct": overall_pct,
            "class_gaps": class_gaps,
            "primitive_gaps": prim_gaps,
            "global_gaps": global_uncovered,
        }
        Path(args.json).write_text(json.dumps(payload, indent=2))
        print(f"Wrote {args.json}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
