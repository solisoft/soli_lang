#!/usr/bin/env python3
"""Regenerate the benchmark tables in both documentation surfaces from raw results.

The two pages are data, not prose-with-numbers-typed-in: hand-editing them is how
a benchmark page goes stale and starts lying. Run the four engines, then run this.

    cd bench/cross-language
    soli --vm bench_all.sl        > /tmp/r_soli.txt
    ruby       bench_all.rb       > /tmp/r_rb.txt      # ruby 4.x
    ruby --yjit bench_all.rb      > /tmp/r_yjit.txt
    ruby --zjit bench_all.rb      > /tmp/r_zjit.txt
    python3 render_docs.py /tmp/r_soli.txt /tmp/r_rb.txt /tmp/r_yjit.txt /tmp/r_zjit.txt

Only the table bodies and the summary rows are rewritten; every word of prose in
both files is left exactly as it is, so editorial changes survive a re-run.
"""

import math
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
MD = REPO / "www/docs/benchmarks.md"
SLV = REPO / "www/app/views/docs/getting-started/benchmarks.html.slv"

# Category order on the page: biggest Soli win first, so a reader meets the
# honest headline (where Soli wins and why) before the losses.
ORDER = ["Aggregate", "String", "Array", "DateTime", "Hash", "Duration", "Numeric", "Control"]


def load(path):
    out = {}
    for line in Path(path).read_text().splitlines():
        parts = line.strip().split("|")
        if len(parts) == 3:
            out[(parts[0], parts[1])] = float(parts[2])
    return out


def geomean(values):
    return math.exp(sum(math.log(max(v, 1e-9)) for v in values) / len(values))


def collect(soli, rb, yj, zj):
    """Group rows by category, each sorted best-Soli-ratio first."""
    cats = {}
    for key, s in soli.items():
        cat, op = key
        if key not in rb or key not in yj or key not in zj:
            raise SystemExit(f"missing Ruby result for {cat}|{op} — suites are out of sync")
        best = min(rb[key], yj[key], zj[key])
        ratio = s / best if best > 0 else 1.0
        cats.setdefault(cat, []).append((op, s, rb[key], yj[key], zj[key], ratio))
    for rows in cats.values():
        rows.sort(key=lambda r: r[5])
    unknown = set(cats) - set(ORDER)
    if unknown:
        raise SystemExit(f"new category {unknown} — add it to ORDER in render_docs.py")
    return cats


def summary_lines(cats):
    """(category, ratio, wins, total) plus the overall row."""
    out = []
    every = []
    for cat in ORDER:
        rows = cats.get(cat)
        if not rows:
            continue
        ratios = [r[5] for r in rows]
        every.extend(ratios)
        out.append((cat, geomean(ratios), sum(1 for r in ratios if r < 1), len(ratios)))
    out.sort(key=lambda r: r[1])
    return out, geomean(every), sum(1 for r in every if r < 1), len(every)


def verdict(ratio):
    """Phrase a ratio in the direction that reads naturally, never as spin."""
    if ratio < 1:
        return f"Soli **{1 / ratio:.1f}x faster**", True
    return f"Ruby {ratio:.1f}x faster", False


def render_md(cats):
    text = MD.read_text()
    rows, overall, wins, total = summary_lines(cats)

    body = []
    for cat, ratio, w, n in rows:
        phrase, _ = verdict(ratio)
        body.append(f"| {cat} | {ratio:.2f}x — {phrase} | {w}/{n} |")
    body.append(f"| **Overall** | **{overall:.2f}x** | **{wins}/{total}** |")
    text = replace_table(text, "| Category | Geometric mean | Soli wins |", body)

    for cat in ORDER:
        if cat not in cats:
            continue
        body = []
        for op, s, r, y, z, ratio in cats[cat]:
            cell = f"**{ratio:.2f}x**" if ratio < 1 else f"{ratio:.2f}x"
            body.append(f"| `{op}` | {s:.3f} | {r:.3f} | {y:.3f} | {z:.3f} | {cell} |")
        text = replace_table(text, "| Operation | Soli | Ruby | +YJIT | +ZJIT | Ratio |", body, cat)

    MD.write_text(text)
    return overall, wins, total, rows


def replace_table(text, header, body, after_heading=None):
    """Swap the rows under `header` (optionally the copy below `## after_heading`)."""
    start = 0
    if after_heading:
        start = text.index(f"\n## {after_heading}\n")
    head_at = text.index(header, start)
    sep_end = text.index("\n", text.index("\n", head_at) + 1) + 1
    end = sep_end
    while end < len(text) and text[end] == "|":
        end = text.index("\n", end) + 1
    return text[:sep_end] + "\n".join(body) + "\n" + text[end:]


TD = '<td class="py-3 px-4 text-right">'


def render_slv(cats):
    text = SLV.read_text()
    rows, overall, wins, total = summary_lines(cats)

    body = []
    for cat, ratio, w, n in rows:
        phrase, good = verdict(ratio)
        phrase = phrase.replace("**", "").replace("faster", "faster")
        colour = "text-emerald-400" if good else "text-amber-300"
        strong = f"<strong>{1 / ratio:.1f}&times; faster</strong>" if good else f"{ratio:.1f}&times; faster"
        who = "Soli" if good else "Ruby"
        body.append(
            f'<tr><td class="py-3 px-4 text-white">{cat}</td>'
            f'<td class="py-3 px-4 text-right {colour}">{ratio:.2f}&times; &mdash; {who} {strong}</td>'
            f'<td class="py-3 px-4 text-right">{w}/{n}</td></tr>'
        )
    body.append(
        f'<tr class="bg-white/5"><td class="py-3 px-4 text-white font-bold">Overall</td>'
        f'<td class="py-3 px-4 text-right text-white font-bold">{overall:.2f}&times;</td>'
        f'<td class="py-3 px-4 text-right text-white font-bold">{wins}/{total}</td></tr>'
    )
    text = replace_rows(text, '<th class="py-3 px-4">Category</th>', body)

    for cat in ORDER:
        if cat not in cats:
            continue
        body = []
        for op, s, r, y, z, ratio in cats[cat]:
            cell = (
                f'<td class="py-3 px-4 text-right text-emerald-400 font-semibold">{ratio:.2f}&times;</td>'
                if ratio < 1
                else f'<td class="py-3 px-4 text-right text-gray-400">{ratio:.2f}&times;</td>'
            )
            body.append(
                f'<tr><td class="py-3 px-4"><code class="text-cyan-400">{op}</code></td>'
                f"{TD}{s:.3f}</td>{TD}{r:.3f}</td>{TD}{y:.3f}</td>{TD}{z:.3f}</td>{cell}</tr>"
            )
        anchor = f'<h2 class="text-2xl font-bold text-white mb-3">{cat}</h2>'
        text = replace_rows(text, '<th class="py-3 px-4">Operation</th>', body, anchor)

    SLV.write_text(text)


def replace_rows(text, header_cell, body, after=None):
    start = text.index(after) if after else 0
    head_at = text.index(header_cell, start)
    tbody_end = text.index("\n", text.index("<tbody", head_at)) + 1
    end = text.index("</tbody>", tbody_end)
    return text[:tbody_end] + "\n".join(body) + "\n" + text[end:]


def main():
    if len(sys.argv) != 5:
        raise SystemExit(__doc__)
    cats = collect(*(load(p) for p in sys.argv[1:5]))
    overall, wins, total, rows = render_md(cats)
    render_slv(cats)
    print(f"wrote {MD.relative_to(REPO)} and {SLV.relative_to(REPO)}")
    for cat, ratio, w, n in rows:
        print(f"  {cat:<10} {ratio:>6.2f}x  {w}/{n}")
    print(f"  {'OVERALL':<10} {overall:>6.2f}x  {wins}/{total}")


if __name__ == "__main__":
    main()
