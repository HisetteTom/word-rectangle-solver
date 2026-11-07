#!/usr/bin/env bash
# Usage: scripts/bench.sh [all|quick|heavy|B<n>] [--release] [--solver NAME] [--runs N] [--label TEXT]
#   default: all benchmarks, `fast` profile, trie solver (the CLI default).
#   quick = B1–B3 (seconds): 1 warmup run (shown) + N timed runs (default 5).
#   heavy = B4+ (minutes): ONE timed run each, no warmup.
#   --release  : build with the full release profile (LTO), use for journal numbers.
#   --label    : name of this step in the charts (default: solver name),
#                e.g. --label "trie + masks". Use a new label for every improvement.
# Appends one line per benchmark to results/bench-log.csv, then regenerates the charts.
#   search_s = median search time reported by the binary (excludes dictionary load)
#   mean_s   = mean wall time of the whole process
set -euo pipefail
cd "$(dirname "$0")/.."

# colors only when writing to a terminal (and NO_COLOR is unset)
if [[ -t 1 && -z ${NO_COLOR:-} ]]; then
  B=$'\e[1m' D=$'\e[2m' R=$'\e[0m' CY=$'\e[36m' GR=$'\e[32m' YE=$'\e[33m' RD=$'\e[31m'
else
  B= D= R= CY= GR= YE= RD=
fi

which_b=all; profile=fast; solver=trie; runs=5; label=
while [[ $# -gt 0 ]]; do
  case "$1" in
    --release) profile=release ;;
    --solver)  solver="$2"; shift ;;
    --runs)    runs="$2"; shift ;;
    --label)   label="$2"; shift ;;
    B*|all|quick|heavy) which_b="$1" ;;
    *) echo "${RD}unknown arg $1${R}"; exit 1 ;;
  esac
  shift
done
label=${label:-$solver}
[[ $label == *,* ]] && { echo "${RD}--label must not contain commas${R}"; exit 1; }

cargo build --quiet --profile "$profile"
BIN="target/$profile/wordrect"

# id | command args | description | heavy (non-empty = run once)
# NEVER change an existing ID's task: old rows in the log must stay comparable. Add new IDs.
BENCHES=(
  "B1|count --rows 3 --cols 3|count all 3x3 (exhaustive)|"
  "B2|find --rows 5 --cols 6|first 5x6|"
  "B3|find --rows 6 --cols 6|first 6x6|"
  # heavy suite, sized at "trie + masks" (~5–10 min total there)
  "B4|count --rows 4 --cols 4|count all 4x4 (exhaustive)|heavy"
  "B5|count --rows 4 --cols 5|count all 4x5 (exhaustive)|heavy"
  "B6|count --rows 3 --cols 7|count all 3x7 (exhaustive)|heavy"
  "B7|find --rows 7 --cols 7|first 7x7|heavy"
  "B8|find --rows 6 --cols 7|first 6x7|heavy"
  "B9|find --rows 5 --cols 9|first 5x9|heavy"
)

commit=$(git rev-parse --short HEAD 2>/dev/null || echo unknown)
[[ -n "$(git status --porcelain --untracked-files=no -- src Cargo.toml 2>/dev/null)" ]] && commit="$commit-dirty"
[[ $commit == *-dirty ]] && echo "${YE}warning: uncommitted changes in src/ — commit first so the log points at real code${R}"
log=results/bench-log.csv
[[ -f $log ]] || echo "date,commit,profile,solver,bench,args,mean_s,nodes,result,search_s,label" > "$log"

search_time() { grep -o 'time=[0-9.]*' <<< "$1" | head -1 | cut -d= -f2; }

for b in "${BENCHES[@]}"; do
  IFS='|' read -r id args desc heavy <<< "$b"
  case $which_b in
    all) ;;
    quick) [[ -z $heavy ]] || continue ;;
    heavy) [[ -n $heavy ]] || continue ;;
    *) [[ $which_b == "$id" ]] || continue ;;
  esac
  echo "${B}${CY}=== $id: $desc${R}   ${D}($BIN $args --solver $solver)  [$label]${R}"

  # first run: shown, gives nodes + result (warmup for quick, THE measurement for heavy)
  s=$(date +%s%N); out=$("$BIN" $args --solver "$solver" --stats); e=$(date +%s%N)
  echo "$out" | tail -n +3 | while IFS= read -r line; do
    case "$line" in
      found*|solutions=*) echo "${GR}${line}${R}" ;;
      "no solution"*)     echo "${RD}${line}${R}" ;;
      "#"*)               echo "${D}${line}${R}" ;;
      "  "*)              echo "${B}${GR}${line}${R}" ;;
      *)                  echo "$line" ;;
    esac
  done
  nodes=$(grep -o 'nodes=[0-9]*' <<< "$out" | head -1 | cut -d= -f2)
  result=$(grep -E '^(solutions=|found|no solution)' <<< "$out" | head -1 | tr ',' ' ' | sed 's/ (validated)://')

  if [[ -n $heavy ]]; then
    n=1; wall_ns=$((e - s)); searches=("$(search_time "$out")")
  else
    n=$runs; wall_ns=0; searches=()
    for ((i = 0; i < n; i++)); do
      s=$(date +%s%N); o=$("$BIN" $args --solver "$solver" --stats); e=$(date +%s%N)
      wall_ns=$((wall_ns + e - s))
      searches+=("$(search_time "$o")")
    done
  fi
  mean=$(awk -v t="$wall_ns" -v n="$n" 'BEGIN { printf "%.4f", t / n / 1e9 }')
  search=$(printf '%s\n' "${searches[@]}" | sort -g | awk '{ a[NR] = $1 } END { m = (NR % 2) ? a[(NR + 1) / 2] : (a[NR / 2] + a[NR / 2 + 1]) / 2; printf "%.6f", m }')
  echo "search (median of $n): ${B}${YE}${search}s${R}   ${D}wall mean ${mean}s${R}"
  echo "$(date +%F),$commit,$profile,$solver,$id,$args,$mean,$nodes,$result,$search,$label" >> "$log"
  echo
done
echo "${D}logged to $log${R}"
python3 scripts/charts.py
