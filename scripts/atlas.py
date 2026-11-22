#!/usr/bin/env python3
"""The atlas: for every grid size R×C (R <= C; C×R has the same answers by transposition),
what does the ENABLE dictionary allow?

  1. exhaustive `count` with a time budget  -> exact number of rectangles
  2. if that times out: `find`               -> "exists" (first grid) or "none" (proven)
  3. if that times out too                   -> "unknown" (the frontier)

Results go to results/atlas.csv (one row per size, resumable: sizes already there are skipped
unless --redo). `scripts/charts.py` draws results/charts/atlas.svg from it.

Sizes with at most RARE solutions also get every grid saved (`--print`), for the gallery chart.
Usage: python3 scripts/atlas.py [--max-rows 8] [--max-cols 15] [--count-s 120] [--find-s 600] [--redo]
"""

import csv
import re
import socket
import subprocess
import sys
import time
from datetime import date
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "results" / "atlas.csv"
BIN = ROOT / "target" / "release" / "wordrect"
FIELDS = ["rows", "cols", "status", "solutions", "grid", "time_s", "nodes", "method", "commit", "date", "machine"]


def arg(name: str, default: int) -> int:
    return int(sys.argv[sys.argv.index(name) + 1]) if name in sys.argv else default


def run(cmd: str, r: int, c: int, budget: int):
    """-> (stdout, seconds) or (None, budget) on timeout."""
    s = time.perf_counter()
    try:
        out = subprocess.run([str(BIN), cmd, "--rows", str(r), "--cols", str(c), "--stats"], cwd=ROOT,
                             capture_output=True, text=True, timeout=budget, check=True).stdout
    except subprocess.TimeoutExpired:
        return None, budget
    return out, time.perf_counter() - s


RARE = 9  # sizes with at most this many solutions get all their grids saved (the gallery)


def rare_grids(r: int, c: int) -> str:
    out = subprocess.run([str(BIN), "count", "--rows", str(r), "--cols", str(c), "--print"], cwd=ROOT,
                         capture_output=True, text=True, check=True).stdout
    return " ".join(l for l in out.splitlines() if re.fullmatch(r"[a-z]+(/[a-z]+)+", l))


def main():
    max_r, max_c = arg("--max-rows", 8), arg("--max-cols", 15)
    count_s, find_s = arg("--count-s", 120), arg("--find-s", 600)
    subprocess.run(["cargo", "build", "--quiet", "--release"], cwd=ROOT, check=True)
    commit = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout.strip()
    rows = {}
    if OUT.exists() and "--redo" not in sys.argv:
        with OUT.open(newline="") as f:
            rows = {(int(x["rows"]), int(x["cols"])): x for x in csv.DictReader(f)}
    for r in range(2, max_r + 1):
        for c in range(r, max_c + 1):
            if (r, c) in rows and rows[(r, c)]["status"] != "unknown":
                x = rows[(r, c)]
                if x["status"] == "counted" and 0 < int(x["solutions"]) <= RARE and not x["grid"]:
                    x["grid"] = rare_grids(r, c)
                    print(f"{r}x{c}: grids {x['grid']}", flush=True)
                    with OUT.open("w", newline="") as f:
                        w = csv.DictWriter(f, FIELDS)
                        w.writeheader()
                        w.writerows(rows[k] for k in sorted(rows))
                continue
            rec = {"rows": r, "cols": c, "commit": commit, "date": date.today().isoformat(), "machine": socket.gethostname(),
                   "solutions": "", "grid": "", "nodes": ""}
            out, t = run("count", r, c, count_s)
            if out is not None:
                n = int(re.search(r"solutions=(\d+)", out)[1])
                rec.update(status="none" if n == 0 else "counted", solutions=n, method="count", time_s=f"{t:.3f}",
                           nodes=re.search(r"nodes=(\d+)", out)[1])
            else:
                out, t = run("find", r, c, find_s)
                if out is None:
                    rec.update(status="unknown", method=f"count>{count_s}s find>{find_s}s", time_s=f"{find_s}")
                elif "no solution" in out:
                    rec.update(status="none", solutions=0, method="find", time_s=f"{t:.3f}", nodes=re.search(r"nodes=(\d+)", out)[1])
                else:
                    grid = [l.replace(" ", "").lower() for l in out.splitlines() if re.fullmatch(r"\s+[A-Z]( [A-Z])*", l)]
                    rec.update(status="exists", method="find", time_s=f"{t:.3f}", grid="/".join(grid),
                               nodes=re.search(r"nodes=(\d+)", out)[1])
            if rec["status"] == "counted" and int(rec["solutions"]) <= RARE:
                rec["grid"] = rare_grids(r, c)
            rows[(r, c)] = rec
            print(f"{r}x{c}: {rec['status']:8} {rec['solutions']!s:>12} {rec['time_s']:>9}s {rec['grid']}", flush=True)
            with OUT.open("w", newline="") as f:  # rewrite every time: resumable
                w = csv.DictWriter(f, FIELDS)
                w.writeheader()
                w.writerows(rows[k] for k in sorted(rows))
    # symmetric word squares (row i = column i), a special case of the n×n squares above
    sq = []
    for n in range(2, max_r + 1):
        t0 = time.perf_counter()
        out = subprocess.run([str(BIN), "squares", "--rows", str(n)], cwd=ROOT, capture_output=True, text=True, check=True).stdout
        sq.append({"n": n, "squares": int(re.search(r"squares=(\d+)", out)[1]), "time_s": f"{time.perf_counter() - t0:.3f}",
                   "commit": commit, "date": date.today().isoformat(), "machine": socket.gethostname()})
        print(f"{n}x{n} symmetric squares: {sq[-1]['squares']}", flush=True)
    with (ROOT / "results" / "squares.csv").open("w", newline="") as f:
        w = csv.DictWriter(f, list(sq[0]))
        w.writeheader()
        w.writerows(sq)
    subprocess.run([sys.executable, str(ROOT / "scripts" / "charts.py")], cwd=ROOT)


if __name__ == "__main__":
    main()
