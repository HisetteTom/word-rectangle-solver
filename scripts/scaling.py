#!/usr/bin/env python3
"""Thread scaling of the current code: search time vs --threads, for a few benchmarks.

Writes results/scaling.csv (replaced on every run: it describes the code at HEAD);
`scripts/charts.py` draws results/charts/scaling.svg from it.
Usage: python3 scripts/scaling.py [--threads 1,2,4,8,12,16,24,32]
"""

import csv
import os
import re
import socket
import statistics
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "results" / "scaling.csv"
BIN = ROOT / "target" / "fast" / "wordrect"
# (id, args): exhaustive tasks only (a `find` stops early, so its scaling is noisy and shape-dependent)
TASKS = [("B5", "count --rows 4 --cols 5"), ("B11", "count --rows 4 --cols 6"), ("B13", "find --rows 5 --cols 10")]


def main():
    ts = [int(x) for x in (sys.argv[sys.argv.index("--threads") + 1] if "--threads" in sys.argv else "1,2,4,8,12,16,24,32").split(",")]
    subprocess.run(["cargo", "build", "--quiet", "--profile", "fast"], cwd=ROOT, check=True)
    commit = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout.strip()
    rows = []
    for bid, args in TASKS:
        for t in ts:
            runs = 1 if t <= 4 else 3
            secs = []
            for _ in range(runs):
                out = subprocess.run([str(BIN), *args.split(), "--threads", str(t), "--stats"], cwd=ROOT,
                                     capture_output=True, text=True, check=True).stdout
                secs.append(float(re.search(r"time=([0-9.]+)", out)[1]))
            med = statistics.median(secs)
            print(f"{bid} {args:26} threads={t:>2}  {med:.3f}s", flush=True)
            rows.append({"bench": bid, "args": args, "threads": t, "search_s": f"{med:.6f}", "commit": commit,
                         "machine": socket.gethostname(), "cores": os.cpu_count()})
    with OUT.open("w", newline="") as f:
        w = csv.DictWriter(f, list(rows[0]))
        w.writeheader()
        w.writerows(rows)
    subprocess.run([sys.executable, str(ROOT / "scripts" / "charts.py")], cwd=ROOT)


if __name__ == "__main__":
    main()
