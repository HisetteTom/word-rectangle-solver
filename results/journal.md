# Optimization journal

One entry per change, failed experiments included. Raw numbers: `results/bench-log.csv`.

> ⚠️ "nodes" for the **naive** solver = number of *partial grids* (one node per placed row word),
> not per cell. It is not directly comparable to the cell-by-cell trie solver's nodes.

### 2026-11-07: Level 0 — naive baseline

**Idea:** whole row words from a list, column prefixes checked against a `HashSet<String>`
of all prefixes (allocates a `String` per column per check). Reference for correctness and speed.

| Benchmark | Time (fast profile) | Nodes | Result |
|---|---|---|---|
| B1 count 3×3 | ~9.3 s | 2,034,829 | 1,909,732 solutions |
| B2 find 5×6  | ~2.4 s | 1,919 | `aahing/acedia/ramees/greave/hisses` |
| B3 find 6×6  | ~5.4 s | 4,505 | `aahing/agonal/hoodie/indole/nailed/gleeds` |

`count 4x4` does not finish in 2 minutes with this solver.

**Verdict:** kept forever as the reference. Never optimize `src/naive.rs`.
