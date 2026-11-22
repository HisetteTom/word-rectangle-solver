#!/usr/bin/env python3
"""Thread timeline of one parallel `find`, with and without splitting long jobs.

Builds the probe binary (`--features probe`, separate target dir), runs the task twice
(`WR_NO_SPLIT=1` turns splitting off in probe builds), writes results/timeline.csv (replaced every
run: it describes HEAD), then redraws the charts (`results/charts/timeline.svg`).
Usage: python3 scripts/timeline.py [--rows 7 --cols 7]
"""

import csv
import os
import re
import statistics
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "results" / "timeline.csv"
TARGET = Path("/tmp/wordrect-probe-target")


def arg(name: str, default: int) -> int:
    return int(sys.argv[sys.argv.index(name) + 1]) if name in sys.argv else default


def run(r: int, c: int, split: bool):
    env = {**os.environ}
    if not split:
        env["WR_NO_SPLIT"] = "1"
    out = subprocess.run([str(TARGET / "fast" / "wordrect"), "find", "--rows", str(r), "--cols", str(c), "--stats"], cwd=ROOT,
                         env=env, capture_output=True, text=True, check=True).stdout
    evs = [(int(m[1]), m[2], float(m[3]), float(m[4]), int(m[5])) for m in re.finditer(r"^# ev (\d+) (\S+) ([\d.]+) ([\d.]+) (\d+)$", out, re.M)]
    return evs, float(re.search(r"time=([0-9.]+)", out)[1])


def main():
    r, c = arg("--rows", 7), arg("--cols", 7)
    env = {**os.environ, "CARGO_TARGET_DIR": str(TARGET)}
    subprocess.run(["cargo", "build", "--quiet", "--profile", "fast", "--features", "probe"], cwd=ROOT, env=env, check=True)
    commit = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout.strip()
    rows = []
    for split in (False, True):
        # 5 runs, keep the median one (timelines are noisy)
        runs = sorted((run(r, c, split) for _ in range(5)), key=lambda x: x[1])
        evs, t = runs[2]
        lanes = {lane: i for i, lane in enumerate(sorted({e[0] for e in evs}))}
        for lane, kind, s, e, rank in evs:
            rows.append({"task": f"find --rows {r} --cols {c}", "split": int(split), "lane": lanes[lane], "kind": kind,
                         "start_us": s, "end_us": e, "job": rank >> 40, "commit": commit})
        print(f"split={split}: {len(evs)} events, search {t * 1e3:.1f} ms (median of 5)", flush=True)
    with OUT.open("w", newline="") as f:
        w = csv.DictWriter(f, list(rows[0]))
        w.writeheader()
        w.writerows(rows)
    subprocess.run([sys.executable, str(ROOT / "scripts" / "charts.py")], cwd=ROOT)


if __name__ == "__main__":
    main()
