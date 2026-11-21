#!/usr/bin/env python3
"""Where the search spends its nodes: per-cell node counts of the current kernel.

Builds the probe binary (`--features probe`, separate target dir so the benchmark build stays
untouched), runs a few tasks, writes results/heatmap.csv (replaced every run: it describes HEAD),
then redraws the charts (`results/charts/heatmap.svg`).
Usage: python3 scripts/heatmap.py
"""

import csv
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "results" / "heatmap.csv"
TARGET = Path("/tmp/wordrect-probe-target")
# find tasks must be exhaustive ("prove none"): a find that stops early would show a partial picture
TASKS = ["count --rows 4 --cols 5", "count --rows 5 --cols 5", "count --rows 5 --cols 6", "find --rows 6 --cols 9"]


def main():
    env = {**__import__("os").environ, "CARGO_TARGET_DIR": str(TARGET)}
    subprocess.run(["cargo", "build", "--quiet", "--profile", "fast", "--features", "probe"], cwd=ROOT, env=env, check=True)
    commit = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout.strip()
    rows = []
    for args in TASKS:
        out = subprocess.run([str(TARGET / "fast" / "wordrect"), *args.split(), "--stats"], cwd=ROOT,
                             capture_output=True, text=True, check=True).stdout
        r = int(re.search(r"--rows (\d+)", args)[1])
        for m in re.finditer(r"^# cell (\S+) (\d+) (\d+)$", out, re.M):
            row = r - 1 if m[1] == "L" else int(m[1])
            rows.append({"task": args, "row": row, "col": int(m[2]), "nodes": int(m[3]), "kernel": "last" if m[1] == "L" else "main", "commit": commit})
        print(f"{args}: {sum(x['nodes'] for x in rows if x['task'] == args):,} nodes", flush=True)
    with OUT.open("w", newline="") as f:
        w = csv.DictWriter(f, list(rows[0]))
        w.writeheader()
        w.writerows(rows)
    subprocess.run([sys.executable, str(ROOT / "scripts" / "charts.py")], cwd=ROOT)


if __name__ == "__main__":
    main()
