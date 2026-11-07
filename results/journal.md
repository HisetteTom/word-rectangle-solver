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

### 2026-11-07: Level 1 — trie + masks (`60c0927`)

**Idea:** replace whole-word placement + `HashSet<String>` prefix checks with one per-length trie
for rows and one for columns, each node carrying a 26-bit child mask. Fill cell by cell (row-major);
legal letters = `row_mask & col_mask`, iterated with `trailing_zeros` / `x & (x - 1)`.
No allocation in the hot loop, dead branches die at the first bad letter instead of after a whole row.

Times = median search time (dictionary load excluded), `fast` profile.

| Benchmark | Before (naive) | After (trie) | Change |
|---|---|---|---|
| B1 count 3×3 | 11.85 s | 77.1 ms | **154× faster** |
| B2 find 5×6 | 2.34 s | 0.90 ms | **2,600× faster** |
| B3 find 6×6 | 5.65 s | 1.91 ms | **2,958× faster** |
| B1 nodes/sec | 0.17 M | 52 M | 300× (nodes not comparable: cells vs rows) |

Same 1,909,732 solutions for 3×3; same first 5×6 and 6×6 grids as naive (both search lexicographically).
Cross-checked in `tests/cross_check.rs`.

**Verdict:** kept, now the default solver. B2/B3 are now ~1 ms → time to add bigger benchmarks (B4+).
