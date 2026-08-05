#!/usr/bin/env python3
"""Parse a sweep log (+ oha JSON dir) into ranked tables ready to paste into the docs."""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

# Lines look like:
#   soli        35,969 req/s  p99    6.54ms  CPU/req    226us  (+solidb 130=356us sys)
#   rails        9,150 req/s  p99   37.48ms  CPU/req  1,393us
LINE = re.compile(
    r"^\s*(?P<stack>\S+)\s+"
    r"(?P<rps>[\d,]+)\s+req/s\s+"
    r"p99\s+(?P<p99>[\d.]+)ms\s+"
    r"CPU/req\s+(?P<cpu>[\d,]+)us"
    r"(?:\s+\(\+solidb\s+(?P<dbc>[\d,]+)=(?P<sys>[\d,]+)us sys\))?"
)

SECTION = re.compile(r"^###\s+(?P<title>.+)$")

STACK_LABEL = {
    "soli": "Soli",
    "rails": "Rails + Puma",
    "express": "Express + EJS + Sequelize",
    "adonis": "AdonisJS + Lucid + Edge",
    "laravel": "Laravel + php-fpm",
    "octane": "Laravel + Octane",
    "django": "Django + gunicorn",
    "fastapi": "FastAPI + SQLAlchemy + Jinja2",
    "phoenix": "Phoenix + Ecto + HEEx",
}


def parse_log(text: str) -> list[tuple[str, list[dict]]]:
    sections: list[tuple[str, list[dict]]] = []
    current: str | None = None
    rows: list[dict] = []
    for line in text.splitlines():
        m = SECTION.match(line)
        if m:
            if current is not None:
                sections.append((current, rows))
            current = m.group("title").strip()
            rows = []
            continue
        m = LINE.match(line)
        if m and current is not None:
            d = m.groupdict()
            rows.append(
                {
                    "stack": d["stack"],
                    "rps": int(d["rps"].replace(",", "")),
                    "p99": float(d["p99"]),
                    "cpu": int(d["cpu"].replace(",", "")),
                    "dbc": int(d["dbc"].replace(",", "")) if d["dbc"] else None,
                    "sys": int(d["sys"].replace(",", "")) if d["sys"] else None,
                }
            )
    if current is not None:
        sections.append((current, rows))
    return sections


def fmt_num(n: int) -> str:
    return f"{n:,}"


def fmt_cpu(row: dict) -> str:
    if row["sys"] is not None and row["dbc"] is not None:
        return f"{fmt_num(row['cpu'])} µs ({fmt_num(row['sys'])} incl. SoliDB)"
    return f"{fmt_num(row['cpu'])} µs"


def print_section(title: str, rows: list[dict]) -> None:
    if not rows:
        return
    rails = next((r for r in rows if r["stack"] == "rails"), None)
    rails_rps = rails["rps"] if rails else 1
    ranked = sorted(rows, key=lambda r: -r["rps"])
    print(f"\n## {title}")
    print(f"{'stack':<32} {'req/s':>10} {'p99':>10} {'CPU/req':>28} {'vs Rails':>10}")
    for r in ranked:
        vs = r["rps"] / rails_rps if rails_rps else 0
        label = STACK_LABEL.get(r["stack"], r["stack"])
        mark = " *" if r["stack"] == "soli" else ""
        print(
            f"{label:<32} {fmt_num(r['rps']):>10} {r['p99']:>8.2f}ms "
            f"{fmt_cpu(r):>28} {vs:>8.1f}×{mark}"
        )


def slv_row(r: dict, rails_rps: int, highlight: bool) -> str:
    vs = r["rps"] / rails_rps if rails_rps else 0
    label = STACK_LABEL.get(r["stack"], r["stack"])
    if r["stack"] == "octane":
        label = (
            'Laravel + Octane <span class="text-xs uppercase tracking-wide">reference</span>'
        )
        tr_class = ' class="text-gray-500"'
        td = "py-3 px-4 text-right"
        name_td = f'<td class="py-3 px-4">{label}</td>'
    elif highlight and r["stack"] == "soli":
        tr_class = ' class="bg-white/5"'
        td = "py-3 px-4 text-right text-white"
        name_td = (
            f'<td class="py-3 px-4 text-white font-bold">{label}</td>'
        )
    else:
        tr_class = ""
        td = "py-3 px-4 text-right"
        name_td = f'<td class="py-3 px-4">{label}</td>'

    cpu = fmt_cpu(r).replace("µs", "&micro;s")
    return (
        f"<tr{tr_class}>{name_td}"
        f'<td class="{td}">{fmt_num(r["rps"])}</td>'
        f'<td class="{td}">{r["p99"]:.2f} ms</td>'
        f'<td class="{td}">{cpu}</td>'
        f'<td class="{td}">{vs:.1f}&times;</td></tr>'
    )


def emit_slv_tables(sections: list[tuple[str, list[dict]]], out: Path) -> None:
    chunks = []
    for title, rows in sections:
        if not rows:
            continue
        rails = next((r for r in rows if r["stack"] == "rails"), None)
        rails_rps = rails["rps"] if rails else 1
        ranked = sorted(rows, key=lambda r: -r["rps"])
        body = "\n".join(slv_row(r, rails_rps, True) for r in ranked)
        chunks.append(f"<!-- {title} -->\n{body}\n")
    out.write_text("\n".join(chunks), encoding="utf-8")
    print(f"wrote SLV row fragments to {out}", file=sys.stderr)


def main() -> int:
    if len(sys.argv) < 2:
        print(f"usage: {sys.argv[0]} <session.log> [out-fragments.html]", file=sys.stderr)
        return 2
    log = Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace")
    sections = parse_log(log)
    if not sections:
        print("no sections parsed — is this a sweep log?", file=sys.stderr)
        return 1
    for title, rows in sections:
        print_section(title, rows)
    if len(sys.argv) >= 3:
        emit_slv_tables(sections, Path(sys.argv[2]))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
