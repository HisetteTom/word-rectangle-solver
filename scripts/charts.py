#!/usr/bin/env python3
"""Turn results/bench-log.csv into progress charts + results/BENCHMARKS.md.

One horizontal bar chart per benchmark (SVG, renders on GitHub, follows light/dark).
One bar per *label* (= one optimization step), oldest at the top, using the
latest run of that label. Time axis is logarithmic, so 1000x speedups stay readable.
Speedup "x faster" is relative to the first step (the naive baseline).

Usage: python3 scripts/charts.py      (bench.sh calls it automatically)
"""

import csv
import math
import re
from html import escape
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LOG = ROOT / "results" / "bench-log.csv"
CHARTS = ROOT / "results" / "charts"
REPORT = ROOT / "results" / "BENCHMARKS.md"


def describe(args: str) -> str:
    m = re.match(r"(\w+) --rows (\d+) --cols (\d+)", args)
    if not m:
        return args
    cmd, r, c = m.groups()
    return f"count all {r}×{c} (exhaustive)" if cmd == "count" else f"find first {r}×{c}"


def fmt_time(s: float) -> str:
    if s >= 1:
        return f"{s:.2f} s"
    if s >= 1e-3:
        return f"{s * 1e3:.3g} ms"
    return f"{s * 1e6:.3g} µs"


def fmt_tick(s: float) -> str:
    if s >= 1:
        return f"{s:g} s"
    if s >= 1e-3:
        return f"{s * 1e3:g} ms"
    return f"{s * 1e6:g} µs"


def fmt_x(x: float) -> str:
    if x >= 100:
        return f"{x:,.0f}×"
    if x >= 10:
        return f"{x:.1f}×"
    return f"{x:.2f}×"


def load():
    """{bench: [step, ...]} where step = latest row of each label, in first-seen order."""
    benches: dict[str, dict[str, dict]] = {}
    with LOG.open(newline="") as f:
        for row in csv.DictReader(f):
            label = (row.get("label") or "").strip() or row["solver"]
            secs = row.get("search_s") or row["mean_s"]
            try:
                row["t"] = float(secs)
            except ValueError:
                continue
            row["label"] = label
            steps = benches.setdefault(row["bench"], {})
            if label in steps:
                steps[label].update(row)  # keep first-seen position, latest numbers
            else:
                steps[label] = row
    return {b: list(s.values()) for b, s in sorted(benches.items())}


def svg(bench: str, steps: list[dict]) -> str:
    left, right, top, row_h, bar_h = 200, 210, 66, 40, 24
    plot_w = 380
    width = left + plot_w + right
    height = top + row_h * len(steps) + 34

    times = [s["t"] for s in steps]
    lo = 10 ** math.floor(math.log10(min(times)))
    hi = 10 ** math.ceil(math.log10(max(times)))
    if hi <= lo:
        hi = lo * 10
    span = math.log10(hi) - math.log10(lo)

    def x(t: float) -> float:
        return left + max(3.0, (math.log10(t) - math.log10(lo)) / span * plot_w)

    base = times[0]
    best = min(times)
    out = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" '
        f'viewBox="0 0 {width} {height}" font-family="ui-sans-serif,-apple-system,Segoe UI,Helvetica,Arial,sans-serif">',
        """<style>
  .bg{fill:#ffffff} .t{fill:#1f2328} .m{fill:#59636e} .g{stroke:#d1d9e0}
  .bar{fill:#8fb8de} .best{fill:#1f6feb} .x{fill:#1a7f37;font-weight:600}
  @media (prefers-color-scheme: dark){
    .bg{fill:#0d1117} .t{fill:#e6edf3} .m{fill:#9198a1} .g{stroke:#30363d}
    .bar{fill:#2f5f8f} .best{fill:#4493f8} .x{fill:#3fb950}
  }
</style>""",
        f'<rect class="bg" width="{width}" height="{height}" rx="8"/>',
        f'<text class="t" x="20" y="30" font-size="17" font-weight="700">{escape(bench)} · {escape(describe(steps[0]["args"]))}</text>',
        f'<text class="m" x="20" y="50" font-size="12">search time, log scale · shorter is better · speedup vs {escape(steps[0]["label"])}</text>',
    ]
    # decade gridlines
    d = math.log10(lo)
    while d <= math.log10(hi) + 1e-9:
        gx = left + (d - math.log10(lo)) / span * plot_w
        out.append(f'<line class="g" x1="{gx:.1f}" x2="{gx:.1f}" y1="{top - 6}" y2="{height - 28}" stroke-dasharray="2 3"/>')
        out.append(f'<text class="m" x="{gx:.1f}" y="{height - 12}" font-size="11" text-anchor="middle">{fmt_tick(round(10 ** d, 9))}</text>')
        d += 1
    for i, s in enumerate(steps):
        y = top + i * row_h
        cy = y + bar_h / 2 + 4
        bx = x(s["t"])
        cls = "best" if s["t"] == best else "bar"
        out.append(f'<text class="t" x="{left - 12}" y="{cy - 3}" font-size="13" font-weight="600" text-anchor="end">{escape(s["label"])}</text>')
        out.append(f'<text class="m" x="{left - 12}" y="{cy + 11}" font-size="10" text-anchor="end">{escape(s["commit"])} · {escape(s["profile"])}</text>')
        out.append(f'<rect class="{cls}" x="{left}" y="{y}" width="{bx - left:.1f}" height="{bar_h}" rx="3"/>')
        speed = "baseline" if i == 0 else f"{fmt_x(base / s['t'])} faster"
        out.append(
            f'<text class="t" x="{bx + 8:.1f}" y="{cy}" font-size="13">{fmt_time(s["t"])}'
            f'<tspan class="{"m" if i == 0 else "x"}" dx="8">{speed}</tspan></text>'
        )
    out.append("</svg>")
    return "\n".join(out) + "\n"


def report(data: dict[str, list[dict]]) -> str:
    lines = [
        "# Benchmark progress",
        "",
        "Generated by `scripts/charts.py` from [`bench-log.csv`](bench-log.csv) — do not edit by hand.",
        "Each bar is one optimization step (the latest run of its `--label`). Time = median search time",
        "reported by the binary (dictionary load excluded). Details of each step: [`journal.md`](journal.md).",
        "",
        "> ⚠️ *nodes* are not comparable across solvers: naive counts placed row words, trie counts cells.",
        "",
    ]
    for bench, steps in data.items():
        lines += [f"## {bench} · {describe(steps[0]['args'])}", "", f"![{bench}](charts/{bench}.svg)", ""]
        lines += ["| Step | Commit | Date | Profile | Time | vs baseline | vs previous | Nodes | Result |",
                  "|---|---|---|---|---:|---:|---:|---:|---|"]
        prev = None
        for s in steps:
            vb = "—" if prev is None else fmt_x(steps[0]["t"] / s["t"])
            vp = "—" if prev is None else fmt_x(prev / s["t"])
            lines.append(
                f"| {s['label']} | `{s['commit']}` | {s['date']} | {s['profile']} | {fmt_time(s['t'])} "
                f"| {vb} | {vp} | {int(s['nodes']):,} | {s['result']} |"
            )
            prev = s["t"]
        lines.append("")
    return "\n".join(lines)


def main():
    data = load()
    CHARTS.mkdir(parents=True, exist_ok=True)
    for bench, steps in data.items():
        (CHARTS / f"{bench}.svg").write_text(svg(bench, steps))
    REPORT.write_text(report(data))
    print(f"charts: {', '.join(f'results/charts/{b}.svg' for b in data)} + results/BENCHMARKS.md")


if __name__ == "__main__":
    main()
