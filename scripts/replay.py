#!/usr/bin/env python3
"""Replay the whole optimization history on THIS machine.

Every step in results/legacy/bench-log.csv and results/bench-log.csv (one per label, in order;
the latest commit of each label) is rebuilt from its commit (fast profile, in a throwaway git worktree) and run on the CURRENT benchmark list
from scripts/bench.sh. Rows are appended to results/bench-log.csv, then the charts are redrawn.
Use it when the main machine changes: old numbers from another CPU are not comparable.

  naive          -> B1–B3 only (it cannot finish anything bigger)
  single-thread  -> B1–B9      (trie, mrv: the huge suites would take hours)
  parallel       -> everything

Resumable: (label, bench) pairs already in the log are skipped, so after adding a benchmark,
running it again fills the new benchmark in for every step.
Usage: python3 scripts/replay.py [--only LABEL] [--dry-run]
"""

import csv
import re
import shutil
import socket
import statistics
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LEGACY = ROOT / "results" / "legacy" / "bench-log.csv"
LOG = ROOT / "results" / "bench-log.csv"
WORK = Path("/tmp/wr-replay")
HEADER = "date,commit,profile,solver,bench,args,mean_s,nodes,result,search_s,label,machine".split(",")

SKIP = {"B14 baseline (tall count)"}  # a bench-only row: with every step run on B14 it is redundant
RENAME = {"parallel 22 threads": "parallel (all cores)"}  # hephaestus has 32
TIMEOUT = 900


def benches():
    text = (ROOT / "scripts" / "bench.sh").read_text()
    return [(m[1], m[2]) for m in re.finditer(r'^\s*"(B\d+)\|([^|]+)\|', text, re.M)]


def steps():
    """Legacy steps first (in order), then the steps recorded on this machine since (in order)."""
    out: dict[str, dict] = {}
    rows = []
    for path in (LEGACY, LOG):
        if path.exists():
            with path.open(newline="") as f:
                rows += list(csv.DictReader(f))
    for row in rows:
        label = (row.get("label") or "").strip() or row["solver"]
        if label in SKIP or row["commit"].endswith("-dirty"):
            continue
        label = RENAME.get(label, label)
        step = out.setdefault(label, {"label": label})
        step.update(commit=row["commit"], solver=row["solver"])  # latest commit of the label
    return list(out.values())


def wanted(solver: str, bench: str) -> bool:
    n = int(bench[1:])
    return n <= 3 if solver == "naive" else n <= 9 if solver in ("trie", "mrv") else True


def run(binary: Path, cwd: Path, args: str, solver: str):
    cmd = [str(binary), *args.split(), "--solver", solver, "--stats"]
    s = time.perf_counter()
    out = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=TIMEOUT, check=True).stdout
    wall = time.perf_counter() - s
    search = float(re.search(r"time=([0-9.]+)", out)[1])
    nodes = re.search(r"nodes=(\d+)", out)[1]
    res = next(l for l in out.splitlines() if re.match(r"(solutions=|found|no solution)", l))
    return wall, search, nodes, res.replace(",", " ").replace(" (validated):", "")


def main():
    only = sys.argv[sys.argv.index("--only") + 1] if "--only" in sys.argv else None
    dry = "--dry-run" in sys.argv
    machine = socket.gethostname()
    done = set()
    if LOG.exists():
        with LOG.open(newline="") as f:
            done = {(r["label"], r["bench"]) for r in csv.DictReader(f)}
    else:
        LOG.write_text(",".join(HEADER) + "\n")
    WORK.mkdir(exist_ok=True)
    bl = benches()
    for st in steps():
        if only and st["label"] != only:
            continue
        todo = [(b, a) for b, a in bl if wanted(st["solver"], b) and (st["label"], b) not in done]
        print(f"\n=== {st['label']}  ({st['commit']}, {st['solver']})  {' '.join(b for b, _ in todo) or 'nothing to do'}", flush=True)
        if dry or not todo:
            continue
        tree = WORK / f"src-{st['commit']}"
        if not tree.exists():
            subprocess.run(["git", "worktree", "add", "-f", "--detach", str(tree), st["commit"]], cwd=ROOT, check=True, capture_output=True)
        try:
            subprocess.run(["cargo", "build", "--quiet", "--profile", "fast"], cwd=tree, check=True,
                           env={**__import__("os").environ, "CARGO_TARGET_DIR": str(WORK / "target")})
            binary = WORK / f"bin-{st['commit']}"
            shutil.copy(WORK / "target" / "fast" / "wordrect", binary)
            date = subprocess.run(["git", "show", "-s", "--format=%cs", st["commit"]], cwd=ROOT, capture_output=True, text=True).stdout.strip()
            for b, args in todo:
                try:
                    wall, search, nodes, res = run(binary, tree, args, st["solver"])
                    walls, searches = [wall], [search]
                    extra = 4 if search < 0.5 else 2 if search < 5 else 0  # median of 5 / 3 / 1 runs
                    for _ in range(extra):
                        w, s, _, _ = run(binary, tree, args, st["solver"])
                        walls.append(w)
                        searches.append(s)
                except (subprocess.TimeoutExpired, subprocess.CalledProcessError, TypeError, StopIteration) as e:
                    print(f"  {b}: FAILED ({type(e).__name__})", flush=True)
                    continue
                med = statistics.median(searches)
                print(f"  {b:4} {args:26} {med:12.6f}s  x{len(searches)}  {res}", flush=True)
                row = [date, st["commit"], "fast", st["solver"], b, args, f"{statistics.mean(walls):.4f}",
                       nodes, res, f"{med:.6f}", st["label"], machine]
                with LOG.open("a") as f:
                    f.write(",".join(row) + "\n")
        finally:
            subprocess.run(["git", "worktree", "remove", "--force", str(tree)], cwd=ROOT, capture_output=True)
    subprocess.run(["git", "worktree", "prune"], cwd=ROOT)
    if not dry:
        subprocess.run([sys.executable, str(ROOT / "scripts" / "charts.py")], cwd=ROOT)


if __name__ == "__main__":
    main()
