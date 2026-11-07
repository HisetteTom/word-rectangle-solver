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

### 2026-11-07: New heavy suite B4–B9 (`166edb9`)

B2/B3 dropped to ~1 ms with the trie, too short to measure progress. Added a heavy suite sized so
"trie + masks" takes **~4 min total**, each benchmark run once: B4 count 4×4 (38 s), B5 count 4×5 (153 s),
B6 count 3×7 (16 s), B7 find 7×7 (5 s), B8 find 6×7 (4 s), B9 find 5×9 (24 s). Baseline for B4–B9 = trie + masks
(naive cannot finish them). `./wr bench quick` = B1–B3 only, `./wr bench heavy` = B4–B9.
Probed and rejected as too slow for now: find 7×8 and find 6×8 (> 10 min / > 2 min).

### 2026-11-07: Level 2 — compact trie (`d3df964`)

**Idea:** node was `[u32; 26]` + mask = 108 bytes. Now 8 bytes (`mask`, `first`): nodes renumbered in
BFS order so a node's children are contiguous and sorted by letter, child = `first + popcount(mask & ((1 << l) - 1))`.
13× smaller trie → far fewer cache misses. Same nodes visited (pure "faster nodes" change).

| Benchmark | Before (trie + masks) | After (compact trie) | Change |
|---|---|---|---|
| B1 count 3×3 | 77.1 ms | 73.9 ms | 1.04× |
| B2 find 5×6 | 0.90 ms | 0.69 ms | 1.30× |
| B3 find 6×6 | 1.91 ms | 1.41 ms | 1.36× |
| B4 count 4×4 | 37.5 s | 33.7 s | 1.11× |
| B5 count 4×5 | 153.1 s | 129.8 s | 1.18× |
| B6 count 3×7 | 16.3 s | 12.7 s | 1.29× |
| B7 find 7×7 | 5.02 s | 3.96 s | 1.27× |
| B8 find 6×7 | 4.04 s | 3.27 s | 1.24× |
| B9 find 5×9 | 23.8 s | 17.8 s | 1.34× |
| heavy total | 240 s | 201 s | 1.19× |

**Verdict:** kept. Modest, as expected: tries for 4–5 letter words were already small enough to sit in cache;
bigger words (7, 9 letters) gain most. Big wins must now come from **visiting fewer nodes** (Level 3:
smarter cell order / forward checking), not from faster nodes.
