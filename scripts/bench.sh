#!/usr/bin/env bash
# Usage: scripts/bench.sh [B1|B2|B3|all] [--release] [--solver NAME] [--runs N]
#   default: all benchmarks, `fast` profile, naive solver.
#   --release  : build with the full release profile (LTO), use for journal numbers.
# Uses hyperfine if installed, otherwise a simple timing loop.
# Every run appends one line per benchmark to results/bench-log.csv.
set -euo pipefail
cd "$(dirname "$0")/.."

which_b=all; profile=fast; solver=naive; runs=3
while [[ $# -gt 0 ]]; do
  case "$1" in
    --release) profile=release ;;
    --solver)  solver="$2"; shift ;;
    --runs)    runs="$2"; shift ;;
    B*|all)    which_b="$1" ;;
    *) echo "unknown arg $1"; exit 1 ;;
  esac
  shift
done

cargo build --quiet --profile "$profile"
BIN="target/$profile/wordrect"

# id | command args | description
BENCHES=(
  "B1|count --rows 3 --cols 3|count all 3x3 (exhaustive)"
  "B2|find --rows 5 --cols 6|first 5x6"
  "B3|find --rows 6 --cols 6|first 6x6"
)

commit=$(git rev-parse --short HEAD 2>/dev/null || echo unknown)
[[ -n "$(git status --porcelain --untracked-files=no 2>/dev/null)" ]] && commit="$commit-dirty"
log=results/bench-log.csv
[[ -f $log ]] || echo "date,commit,profile,solver,bench,args,mean_s,nodes,result" > "$log"

for b in "${BENCHES[@]}"; do
  IFS='|' read -r id args desc <<< "$b"
  [[ $which_b == all || $which_b == "$id" ]] || continue
  echo "=== $id: $desc   ($BIN $args --solver $solver)"
  # One run with --stats to get nodes + result
  out=$("$BIN" $args --solver "$solver" --stats)
  echo "$out" | tail -n +3
  nodes=$(grep -o 'nodes=[0-9]*' <<< "$out" | head -1 | cut -d= -f2)
  result=$(grep -E '^(solutions=|found|no solution)' <<< "$out" | head -1 | tr ',' ' ')

  if command -v hyperfine >/dev/null; then
    json=$(mktemp)
    hyperfine --warmup 1 --runs "$runs" --export-json "$json" -N "$BIN $args --solver $solver"
    mean=$(grep -o '"mean": *[0-9.e-]*' "$json" | head -1 | grep -o '[0-9.e-]*$')
    rm -f "$json"
  else
    total_ns=0
    for ((i = 0; i < runs; i++)); do
      s=$(date +%s%N); "$BIN" $args --solver "$solver" >/dev/null; e=$(date +%s%N)
      total_ns=$((total_ns + e - s))
    done
    mean=$(awk -v t="$total_ns" -v n="$runs" 'BEGIN { printf "%.4f", t / n / 1e9 }')
    echo "mean over $runs runs: ${mean}s   (install hyperfine for proper stats)"
  fi
  echo "$(date +%F),$commit,$profile,$solver,$id,$args,$mean,$nodes,$result" >> "$log"
  echo
done
echo "logged to $log"
