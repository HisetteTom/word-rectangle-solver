#!/usr/bin/env python3
"""Turn results/bench-log.csv into progress charts + results/BENCHMARKS.md.

One horizontal bar chart per benchmark (SVG, renders on GitHub, follows light/dark).
One bar per *label* (= one optimization step), oldest at the top, using the
latest run of that label. Time axis is logarithmic when steps span > 20x (so 1000x
speedups stay readable), linear from 0 otherwise (so 1.2x gains stay visible).
Speedup "x faster" is relative to the first step (the naive baseline).
Steps whose label says "(failed)" were reverted: drawn grey and hatched, never highlighted.
The highlighted bar is the code that is in the tree now (the latest kept step).
charts/progress.svg is the headline: the whole history on one log-scale line.

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
MACHINE = "hephaestus · Ryzen 9 3950X · 16 cores / 32 threads"
FONT = "ui-sans-serif,-apple-system,Segoe UI,Helvetica,Arial,sans-serif"
STYLE = """<style>
  .bg{fill:#ffffff} .t{fill:#1f2328} .m{fill:#59636e} .g{stroke:#d1d9e0}
  .bar{fill:#8fb8de} .best{fill:#1f6feb} .x{fill:#1a7f37;font-weight:600}
  .fail{fill:url(#hatch)} .hs{stroke:#afb8c1} .hb{fill:#eaeef2} .f{fill:#cf222e;font-weight:600}
  .ft{fill:#8c959f} .cur{fill:#1f6feb;font-weight:700}
  .ln{stroke:#1f6feb;fill:none} .lnf{fill:#1f6feb} .ctx{stroke:#afb8c1;fill:none} .ring{stroke:#ffffff}
  .fx{stroke:#8c959f;fill:none} .area{fill:#1f6feb;fill-opacity:.08} .tile{fill:#f6f8fa;stroke:#d1d9e0}
  .big{fill:#1f6feb;font-weight:800}
  @media (prefers-color-scheme: dark){
    .bg{fill:#0d1117} .t{fill:#e6edf3} .m{fill:#9198a1} .g{stroke:#30363d}
    .bar{fill:#2f5f8f} .best{fill:#4493f8} .x{fill:#3fb950}
    .hs{stroke:#484f58} .hb{fill:#21262d} .f{fill:#f85149} .ft{fill:#6e7681} .cur{fill:#4493f8}
    .ln{stroke:#4493f8} .lnf{fill:#4493f8} .ctx{stroke:#484f58} .ring{stroke:#0d1117}
    .fx{stroke:#6e7681} .area{fill:#4493f8;fill-opacity:.10} .tile{fill:#161b22;stroke:#30363d} .big{fill:#4493f8}
  }
</style>"""


def describe(args: str, result: str = "") -> str:
    m = re.match(r"(\w+) --rows (\d+) --cols (\d+)", args)
    if not m:
        return args
    cmd, r, c = m.groups()
    if cmd == "count":
        return f"count all {r}×{c} (exhaustive)"
    if result.startswith("no solution"):
        return f"prove no {r}×{c} exists (exhaustive)"
    return f"find first {r}×{c}"


def fmt_time(s: float) -> str:
    if s >= 100:
        return f"{s:.0f} s"
    if s >= 10:
        return f"{s:.1f} s"
    if s >= 1:
        return f"{s:.2f} s"
    if s >= 1e-3:
        return f"{s * 1e3:.3g} ms"
    return f"{s * 1e6:.3g} µs"


def fmt_tick(s: float) -> str:
    if s >= 1:
        return f"{s:.4g} s"
    if s >= 1e-3:
        return f"{s * 1e3:.4g} ms"
    return f"{s * 1e6:.4g} µs"


def fmt_x(x: float) -> str:
    if x >= 100:
        return f"{x:,.0f}×"
    if x >= 10:
        return f"{x:.1f}×"
    return f"{x:.2f}×"


def failed(label: str) -> bool:
    """Reverted experiments are labelled "... (failed)" (see CLAUDE.md)."""
    return "(failed)" in label


def fmt_speed(r: float) -> str:
    return f"{fmt_x(r)} faster" if r >= 1 else f"{fmt_x(1 / r)} slower"


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
    return {b: list(s.values()) for b, s in sorted(benches.items(), key=lambda kv: int(kv[0][1:]) if kv[0][1:].isdigit() else 0)}


def svg(bench: str, steps: list[dict]) -> str:
    left, right, top, row_h, bar_h = 230, 250, 66, 40, 24
    left = max(left, 32 + round(7.4 * max(len(s["label"]) for s in steps)))  # fit the longest step name
    plot_w = 380
    width = left + plot_w + right
    height = top + row_h * len(steps) + 34

    times = [s["t"] for s in steps]
    # Wide range (> 20x): log scale with decade ticks. Narrow range: linear from 0,
    # otherwise a 1.2x gain would be invisible.
    log_scale = max(times) / min(times) > 20
    if log_scale:
        lo = 10 ** math.floor(math.log10(min(times)))
        hi = 10 ** math.ceil(math.log10(max(times)))
        span = math.log10(hi) - math.log10(lo)
        ticks = [lo * 10**k for k in range(round(span) + 1)]

        def frac(t: float) -> float:
            return (math.log10(t) - math.log10(lo)) / span
    else:
        raw = max(times) / 4
        mag = 10 ** math.floor(math.log10(raw))
        step = next(m * mag for m in (1, 2, 2.5, 5, 10) if m * mag >= raw)
        hi = step * math.ceil(max(times) / step)
        ticks = [step * k for k in range(round(hi / step) + 1)]

        def frac(t: float) -> float:
            return t / hi

    def x(t: float) -> float:
        return left + max(3.0, frac(t) * plot_w)

    base = times[0]
    kept = [i for i, s in enumerate(steps) if not failed(s["label"])]
    current = kept[-1] if kept else None  # the code in the tree now
    out = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" '
        f'viewBox="0 0 {width} {height}" font-family="{FONT}">',
        STYLE,
        '<defs><pattern id="hatch" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">'
        '<rect class="hb" width="6" height="6"/><line class="hs" x1="0" y1="0" x2="0" y2="6" stroke-width="2.5"/></pattern></defs>',
        f'<rect class="bg" width="{width}" height="{height}" rx="8"/>',
        f'<text class="t" x="20" y="30" font-size="17" font-weight="700">{escape(bench)} · {escape(describe(steps[0]["args"], steps[0].get("result", "")))}</text>',
        f'<text class="m" x="20" y="50" font-size="12">search time{", log scale" if log_scale else ""} · shorter is better · speedup vs {escape(steps[0]["label"])} · hatched = failed experiment, reverted</text>',
        f'<text class="m" x="{width - 20}" y="30" font-size="11" text-anchor="end">{escape(MACHINE)}</text>',
    ]
    for tv in ticks:
        gx = left + frac(tv) * plot_w if tv > 0 else left
        out.append(f'<line class="g" x1="{gx:.1f}" x2="{gx:.1f}" y1="{top - 6}" y2="{height - 28}" stroke-dasharray="2 3"/>')
        out.append(f'<text class="m" x="{gx:.1f}" y="{height - 12}" font-size="11" text-anchor="middle">{fmt_tick(round(tv, 9)) if tv > 0 else "0"}</text>')
    for i, s in enumerate(steps):
        y = top + i * row_h
        cy = y + bar_h / 2 + 4
        bx = x(s["t"])
        bad = failed(s["label"])
        cls = "fail" if bad else "best" if i == current else "bar"
        name_cls = "ft" if bad else "t"
        out.append(f'<text class="{name_cls}" x="{left - 12}" y="{cy - 3}" font-size="13" font-weight="600" text-anchor="end">{escape(s["label"])}</text>')
        out.append(f'<text class="m" x="{left - 12}" y="{cy + 11}" font-size="10" text-anchor="end">{escape(s["commit"])} · {escape(s["profile"])}</text>')
        out.append(f'<rect class="{cls}" x="{left}" y="{y}" width="{bx - left:.1f}" height="{bar_h}" rx="3"/>')
        if i == 0:
            tail = '<tspan class="m" dx="8">baseline</tspan>'
        elif bad:
            tail = f'<tspan class="m" dx="8">{fmt_speed(base / s["t"])}</tspan><tspan class="f" dx="8">✗ reverted</tspan>'
        else:
            tail = f'<tspan class="{"x" if base >= s["t"] else "m"}" dx="8">{fmt_speed(base / s["t"])}</tspan>'
        if i == current and i != 0:
            tail += '<tspan class="cur" dx="8">◀ current</tspan>'
        out.append(f'<text class="{name_cls}" x="{bx + 8:.1f}" y="{cy}" font-size="13">{fmt_time(s["t"])}{tail}</text>')
    out.append("</svg>")
    return "\n".join(out) + "\n"


def geomean(xs: list[float]) -> float:
    return math.exp(sum(math.log(v) for v in xs) / len(xs))


def bench_range(ids: list[str]) -> str:
    return ids[0] if len(ids) == 1 else f"{ids[0]}–{ids[-1]}"


def progress(data: dict[str, list[dict]]):
    """Headline chart: one log-scale line through every step.

    Line = geometric-mean speedup over the benchmarks that the longest-running baseline
    with real coverage has (the ones every step after it ran), indexed to that baseline.
    Faint lines = each of those benchmarks alone. Hollow grey dots = failed experiments.
    Tiles on top = current code vs the starting point of each benchmark group.
    """
    # first-seen order in the log = history order
    seen: list[str] = []
    with LOG.open(newline="") as f:
        for row in csv.DictReader(f):
            lab = (row.get("label") or "").strip() or row["solver"]
            if lab not in seen:
                seen.append(lab)
    order = seen
    t = {b: {s["label"]: s["t"] for s in steps} for b, steps in data.items()}
    first = {b: steps[0]["label"] for b, steps in data.items()}
    kept = [l for l in order if not failed(l)]
    current = kept[-1]

    # groups of benchmarks by their baseline step (naive / trie + masks / parallel ...)
    groups: dict[str, list[str]] = {}
    for b in data:
        groups.setdefault(first[b], []).append(b)
    # the line's benchmark set: the group with the most steps that is not naive-only-tiny
    core_base = max(groups, key=lambda g: (len([l for l in order if all(l in t[b] for b in groups[g])]), len(groups[g])) if g != order[0] else (-1, 0))
    core = groups[core_base]
    xs_labels = [l for l in order if all(l in t[b] for b in core)]
    sp = {l: geomean([t[b][core_base] / t[b][l] for b in core]) for l in xs_labels}

    left, right, top, plot_h = 70, 170, 200, 330
    step_w = 30
    plot_w = step_w * (len(xs_labels) - 1)
    width = left + plot_w + right
    height = top + plot_h + 175

    vals = list(sp.values()) + [t[b][core_base] / t[b][l] for b in core for l in xs_labels]
    lo = 10 ** math.floor(math.log10(min(vals)))
    hi = 10 ** math.ceil(math.log10(max(vals) * 1.02))
    span = math.log10(hi) - math.log10(lo)

    def X(i: int) -> float:
        return left + i * step_w

    def Y(v: float) -> float:
        return top + plot_h - (math.log10(v) - math.log10(lo)) / span * plot_h

    out = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}" font-family="{FONT}">',
        STYLE,
        f'<rect class="bg" width="{width}" height="{height}" rx="10"/>',
        '<text class="t" x="24" y="38" font-size="22" font-weight="800">Word rectangle solver · how fast it got</text>',
        f'<text class="m" x="24" y="60" font-size="12.5">every optimization step, re-run on one machine · {escape(MACHINE)}</text>',
    ]
    # tiles: current code vs each group's starting point
    tiles = []
    for g, ids in groups.items():
        if current not in t[ids[0]]:
            continue
        x = geomean([t[b][g] / t[b][current] for b in ids if current in t[b]])
        tiles.append((x, f"faster than {g}", f"{bench_range(ids)} · geometric mean"))
    tw = (width - 48 - 16 * (len(tiles) - 1)) / max(1, len(tiles))
    for k, (x, l1, l2) in enumerate(tiles):
        tx = 24 + k * (tw + 16)
        out.append(f'<rect class="tile" x="{tx:.1f}" y="78" width="{tw:.1f}" height="86" rx="8"/>')
        out.append(f'<text class="big" x="{tx + 18:.1f}" y="122" font-size="34">{fmt_x(x)}</text>')
        out.append(f'<text class="t" x="{tx + 18:.1f}" y="143" font-size="13" font-weight="600">{escape(l1)}</text>')
        out.append(f'<text class="m" x="{tx + 18:.1f}" y="158" font-size="11">{escape(l2)}</text>')

    # grid (decades) + axis title
    k = 0
    while lo * 10**k <= hi * 1.0001:
        v = lo * 10**k
        y = Y(v)
        out.append(f'<line class="g" x1="{left}" x2="{left + plot_w}" y1="{y:.1f}" y2="{y:.1f}" stroke-dasharray="2 3"/>')
        out.append(f'<text class="m" x="{left - 10}" y="{y + 4:.1f}" font-size="11" text-anchor="end">{fmt_x(v).replace(".00", "")}</text>')
        k += 1
    out.append(f'<text class="m" x="24" y="{top - 16}" font-size="12">× faster than {escape(core_base)}, log scale · {bench_range(core)} ({len(core)} benchmarks)</text>')

    # context: each core benchmark alone (kept steps only)
    kx = [(i, l) for i, l in enumerate(xs_labels) if not failed(l)]
    ends = []
    for b in core:
        pts = " ".join(f"{X(i):.1f},{Y(t[b][core_base] / t[b][l]):.1f}" for i, l in kx)
        out.append(f'<polyline class="ctx" points="{pts}" stroke-width="1.2" stroke-linejoin="round"/>')
        ends.append([Y(t[b][core_base] / t[b][kx[-1][1]]) + 3, b])
    ends.sort()
    for k in range(1, len(ends)):  # keep end labels >= 11 px apart
        ends[k][0] = max(ends[k][0], ends[k - 1][0] + 11)
    for y, b in ends:
        out.append(f'<text class="ft" x="{X(kx[-1][0]) + 9:.1f}" y="{y:.1f}" font-size="9.5">{b}</text>')
    # main line + area
    pts = [(X(i), Y(sp[l])) for i, l in kx]
    poly = " ".join(f"{x:.1f},{y:.1f}" for x, y in pts)
    out.append(f'<polygon class="area" points="{pts[0][0]:.1f},{top + plot_h} {poly} {pts[-1][0]:.1f},{top + plot_h}"/>')
    out.append(f'<polyline class="ln" points="{poly}" stroke-width="3" stroke-linejoin="round" stroke-linecap="round"/>')
    for i, l in enumerate(xs_labels):
        x, y = X(i), Y(sp[l])
        if failed(l):
            out.append(f'<circle class="fx ring" cx="{x:.1f}" cy="{y:.1f}" r="4.5" stroke-width="2" fill="none"/>')
            out.append(f'<circle class="fx" cx="{x:.1f}" cy="{y:.1f}" r="4.5" stroke-width="1.6"/>')
        else:
            out.append(f'<circle class="lnf ring" cx="{x:.1f}" cy="{y:.1f}" r="{6.5 if l == current else 4.5}" stroke-width="2"/>')
    # direct label on the current point
    i = xs_labels.index(current)
    out.append(f'<text class="big" x="{X(i) + 40:.1f}" y="{Y(sp[current]) + 2:.1f}" font-size="22">{fmt_x(sp[current])}</text>')
    out.append(f'<text class="m" x="{X(i) + 40:.1f}" y="{Y(sp[current]) + 18:.1f}" font-size="11">geometric mean</text>')
    # x labels (rotated)
    for i, l in enumerate(xs_labels):
        cls = "ft" if failed(l) else "cur" if l == current else "t"
        name = escape(l.replace(" (failed)", "")) + (" ✗" if failed(l) else "")
        out.append(f'<text class="{cls}" font-size="11" text-anchor="end" transform="translate({X(i) + 4:.1f},{top + plot_h + 14}) rotate(-45)">{name}</text>')
    out.append(f'<text class="m" x="24" y="{height - 14}" font-size="11">● kept step   ○ ✗ failed experiment (measured, then reverted)   — thin lines: each benchmark alone · bars per benchmark below</text>')
    out.append("</svg>")
    return "\n".join(out) + "\n"


def report(data: dict[str, list[dict]]) -> str:
    lines = [
        "# Benchmark progress",
        "",
        f"All numbers from one machine: **{MACHINE}**. Every step of the history was rebuilt",
        "from its commit and re-run there (`scripts/replay.py`); runs from the previous machine are",
        "kept in [`legacy/bench-log.csv`](legacy/bench-log.csv) but not charted (not comparable).",
        "",
        "![progress](charts/progress.svg)",
        "",
        "Generated by `scripts/charts.py` from [`bench-log.csv`](bench-log.csv) — do not edit by hand.",
        "Each bar is one optimization step (the latest run of its `--label`). Time = median search time",
        "reported by the binary (dictionary load excluded). Details of each step: [`journal.md`](journal.md).",
        "",
        "> ⚠️ *nodes* are not comparable across solvers: naive counts placed row words, trie counts cells.",
        "",
        "Status: ✅ kept · ❌ failed experiment, reverted (it never stays in the code) · ⭐ the code in the tree now.",
        "*vs previous* compares with the previous **kept** step, the code each experiment started from.",
        "",
    ]
    for bench, steps in data.items():
        lines += [f"## {bench} · {describe(steps[0]['args'], steps[0].get('result', ''))}", "", f"![{bench}](charts/{bench}.svg)", ""]
        lines += ["| | Step | Commit | Date | Profile | Time | vs baseline | vs previous | Nodes | Result |",
                  "|---|---|---|---|---|---:|---:|---:|---:|---|"]
        kept = [i for i, s in enumerate(steps) if not failed(s["label"])]
        current = kept[-1] if kept else None
        prev = None
        for i, s in enumerate(steps):
            bad = failed(s["label"])
            mark = "❌" if bad else "⭐" if i == current else "✅"
            vb = "—" if prev is None else fmt_x(steps[0]["t"] / s["t"])
            vp = "—" if prev is None else fmt_x(prev / s["t"])
            name = f"~~{s['label']}~~" if bad else f"**{s['label']}**" if i == current else s["label"]
            lines.append(
                f"| {mark} | {name} | `{s['commit']}` | {s['date']} | {s['profile']} | {fmt_time(s['t'])} "
                f"| {vb} | {vp} | {int(s['nodes']):,} | {s['result']} |"
            )
            if not bad:
                prev = s["t"]
        lines.append("")
    return "\n".join(lines)


def main():
    data = load()
    CHARTS.mkdir(parents=True, exist_ok=True)
    for bench, steps in data.items():
        (CHARTS / f"{bench}.svg").write_text(svg(bench, steps))
    (CHARTS / "progress.svg").write_text(progress(data))
    REPORT.write_text(report(data))
    print(f"charts: {', '.join(f'results/charts/{b}.svg' for b in data)} + results/BENCHMARKS.md")


if __name__ == "__main__":
    main()
