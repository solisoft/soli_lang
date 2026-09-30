#!/usr/bin/env python3
"""Summarises results/*.jsonl into a Markdown table per stack × spec.

    report.py results/20261001-120000.jsonl [more.jsonl ...]
"""

import json
import statistics
import sys
from collections import defaultdict


def mean(values):
    values = [value for value in values if isinstance(value, (int, float))]
    return statistics.mean(values) if values else None


def fmt(value, pattern):
    return "–" if value is None else pattern.format(value)


def main(paths):
    records = [json.loads(line) for path in paths for line in open(path) if line.strip()]
    if not records:
        sys.exit("no results")
    groups = defaultdict(list)
    for record in records:
        groups[(record["stack"], record["spec"])].append(record)
        groups[(record["stack"], "ALL")].append(record)

    models = sorted({record["model"] for record in records})
    print(f"Model: {', '.join(models)} — {len(records)} trials\n")
    print("| Stack | Spec | Trials | Fully passing | Checks passed | Cost / trial | Wall time | Turns | Lines added |")
    print("|---|---|---:|---:|---:|---:|---:|---:|---:|")
    for (stack, spec), rows in sorted(groups.items(), key=lambda item: (item[0][0], item[0][1] == "ALL", item[0][1])):
        full = sum(row["passed"] == row["total"] for row in rows)
        checks = sum(row["passed"] for row in rows) / max(1, sum(row["total"] for row in rows))
        label = f"**{spec}**" if spec == "ALL" else spec
        print(f"| {stack} | {label} | {len(rows)} | {full}/{len(rows)} ({full / len(rows):.0%}) | {checks:.0%} "
              f"| {fmt(mean(row.get('cost_usd') for row in rows), '${:.2f}')} "
              f"| {fmt(mean(row.get('wall_s') for row in rows), '{:.0f} s')} "
              f"| {fmt(mean(row.get('num_turns') for row in rows), '{:.0f}')} "
              f"| {fmt(mean(row.get('lines_added') for row in rows), '{:.0f}')} |")

    failures = defaultdict(int)
    for record in records:
        for check in record["checks"]:
            if not check["ok"]:
                failures[(record["stack"], record["spec"], check["name"])] += 1
    if failures:
        print("\nMost frequent failing checks:\n")
        for (stack, spec, name), count in sorted(failures.items(), key=lambda item: -item[1])[:15]:
            print(f"- {stack} / {spec} / `{name}` — {count}×")
    errors = [record for record in records if record.get("error")]
    if errors:
        print("\nTrials that did not reach the checks:\n")
        for record in errors:
            print(f"- {record['stack']} / {record['spec']} #{record['trial']}: {record['error']}")


if __name__ == "__main__":
    main(sys.argv[1:])
