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

### 2026-11-07: Level 3 — staircase MRV + frontier forward checking (`072145f`) ❌ reverted

**Idea:** a cell is fillable once its row prefix and column prefix are filled, so fillable cells are the
corners of a staircase. At each node: if any frontier cell has `row_mask & col_mask == 0`, prune; otherwise
branch on the frontier cell with the fewest legal letters (classic "most constrained variable first").

| Benchmark | Before (compact trie) | After (MRV) | Change |
|---|---|---|---|
| B1 count 3×3 | 73.9 ms | 90.7 ms | 0.81× (slower) |
| B3 find 6×6 | 1.41 ms | 0.54 ms | 2.6× (lucky: different first solution) |
| B4 count 4×4 | 33.7 s | 37.8 s | 0.89× — nodes −14% but each node costs more |
| B5 count 4×5 | 129.8 s | 188.8 s | 0.69× |
| B6 count 3×7 | 12.7 s | 25.8 s | 0.49× |
| B9 find 5×9 | 17.8 s | 62.8 s | 0.28× — 3× more nodes |

**Verdict:** reverted. Letter count of a *single cell* is a bad proxy for subtree size: picking a cell with
few letters often commits to a deep, wide subtree. Row-major order already prunes very well because each
row is walked inside one trie (prefix sharing). The frontier check almost never fires: trie nodes below
full depth always have children. Lesson: ordering heuristics need a better cost estimate (e.g. word
counts under the nodes), not raw popcount.

### 2026-11-07: Level 7 — counting without leaf visits (`e12b165`) ✅

**Idea:** `count` gets its own recursion. At the last cell, every letter in `row_mask & col_mask`
completes both the last row and the last column, so that cell contributes `popcount(allowed)`. No node
is visited per solution, no grid writes, no callback.

| Benchmark | Before (compact trie) | After (leaf counting) | Change |
|---|---|---|---|
| B1 count 3×3 | 73.9 ms | 31.5 ms | **2.34×** (1.9 M solutions, leaves dominated) |
| B4 count 4×4 | 33.7 s | 26.8 s | 1.26× |
| B5 count 4×5 | 129.8 s | 120.2 s | 1.08× |
| B6 count 3×7 | 12.7 s | 12.1 s | 1.05× |
| B7–B9 find | 3.96 / 3.27 / 17.8 s | 4.10 / 3.39 / 18.0 s | ≈ (noise; find unchanged) |

**Verdict:** kept. The gain grows with the solutions/nodes ratio. For big counts, most work is in internal
nodes that die out, so the gain stays small.

### 2026-11-07: Level 6 — parallel search on 22 threads (`ecc8a36`) ✅

**Idea:** new default solver `par` (`src/par.rs`), std threads only.
1. Expand the first cells breadth-first, in lexicographic order, into a list of job prefixes.
2. `thread::scope` workers pull job indices from an `AtomicUsize`. This is dynamic load balancing, which
   matters because subtree sizes vary wildly and this CPU mixes P-cores and E-cores.
3. Deterministic `find`: the lowest job index with a solution wins (`fetch_min`). Later jobs are skipped or
   aborted, so the grid is *exactly* the sequential one (checked on B7–B9 and in a new cross-check test
   that forces the parallel path at 1/3/22 threads).

Tuning (probes, jobs per thread):

| jobs/thread | B7 find 7×7 | B8 find 6×7 | B9 find 5×9 | B3 find 6×6 | B6 count 3×7 |
|---|---|---|---|---|---|
| 16 | 3.90 s | 2.50 s | 1.57 s | 2.5 ms | 0.63 s |
| 256 | 3.63 s | 2.47 s | 1.27 s | 3.5 ms | 0.62 s |
| 4096 | 0.65 s | 0.51 s | 1.02 s | 16.7 ms | 0.68 s |
| 65536 | 0.63 s | 0.55 s | 1.26 s | 172 ms | 0.82 s |

With few jobs, the job holding the first 7×7 solution is one huge subtree that a single thread grinds through.
Many small jobs fix that, but the serial job expansion then costs too much on tiny finds. So:
count uses 256/thread, and find uses 4096/thread but first tries a **2M-node sequential search** (~10 ms).
Tiny finds stay sequential.

| Benchmark | Before (leaf counting) | After (parallel) | Change |
|---|---|---|---|
| B1 count 3×3 | 31.5 ms | 7.8 ms | **4.0×** |
| B2 find 5×6 | 0.72 ms | 0.73 ms | ≈ (sequential path) |
| B3 find 6×6 | 1.44 ms | 1.51 ms | ≈ (sequential path) |
| B4 count 4×4 | 26.8 s | 1.48 s | **18.0×** |
| B5 count 4×5 | 120.2 s | 6.43 s | **18.7×** |
| B6 count 3×7 | 12.1 s | 0.63 s | **19.3×** |
| B7 find 7×7 | 4.10 s | 0.70 s | **5.9×** |
| B8 find 6×7 | 3.39 s | 0.63 s | **5.4×** |
| B9 find 5×9 | 18.0 s | 1.06 s | **17.0×** |
| heavy total | 184 s | 10.9 s | **16.9×** |

**Verdict:** kept, now the default. Counting scales ~18–19× on 22 threads (6 P + 8 E + 2 LP-E cores,
hyper-threads), close to the hardware limit. Finds scale less: work before the solution is partly
sequential, and parallel workers also explore jobs *after* the winner. The heavy suite now runs in ~11 s, so the next
big benchmarks (B10+) should be added soon.

### 2026-11-07: note — orientation (R×C vs C×R), not implemented

Probe with `par`: count 4×5 6.48 s vs 5×4 9.40 s, count 3×7 0.64 s vs 7×3 1.38 s, find 6×7 0.46 s vs 7×6 0.95 s,
find 5×6 0.5 ms vs 6×5 2.7 ms, but find 5×9 1.04 s vs 9×5 0.62 s. **Fewer, longer rows win** in 5 of 6 cases.
Every benchmark is already in that orientation, and a transposed `find` returns a different grid. So auto-orient
would gain nothing here. Skipped; no bench row.

### 2026-11-07: Level 2/4 — minimized DAWG (`ad3fb0c`) ❌ and DAWG + row memo (`7a14702`) ❌ reverted

**Idea:** merge trie nodes with identical suffix sets (DAFSA). That gives 6–8× fewer nodes (4-letter words: 6067 → 811;
9-letter: 108k → 18k). Children are shared, so lookups need an `edges` array: `edges[first + popcount(…)]`.
In exchange, a node now means only "what can still follow", so different partial grids can reach the
**same** column-state tuple. Then memoize `tuple → completions` at row starts (per-thread hash map,
u128 packed key, cap of 1 M entries).

| Benchmark | parallel (trie) | DAWG | DAWG + row memo |
|---|---|---|---|
| B1 count 3×3 | 7.8 ms | 6.7 ms | 7.2 ms |
| B4 count 4×4 | 1.48 s | 1.64 s (0.90×) | 2.46 s (0.60×) |
| B5 count 4×5 | 6.43 s | 6.89 s (0.93×) | 8.44 s (0.76×) |
| B6 count 3×7 | 0.63 s | 0.73 s (0.85×) | 0.72 s |
| B7 find 7×7 | 0.70 s | 0.61 s | 0.69 s |
| B9 find 5×9 | 1.06 s | 1.14 s (0.93×) | 1.14 s |

Memo node savings (probe): only **1–9 %** for any start row (1, 2, 3). Tuples of 4–5 column states
almost never repeat, because the merged states are mostly deep (near-final) ones.

**Verdict:** both reverted. The tries already fit in L2, so a smaller automaton buys nothing, and the
extra edge load costs 5–15 %. Memoizing on column states is the wrong granularity for rectangles of
this size.

### 2026-11-07: Level 7 — by-value node kernel (`6da98c6`) ✅

**Profile first** (`perf stat`, sequential `trie`, B4): **~100 instructions and ~37 cycles per node**. Also,
the scheduler ran the single-threaded process on an **E-core** 97.5 % of the time (`cpu_atom`). So sequential
numbers depend on core placement; the parallel runs use every core anyway.

**Idea:** a dedicated hot recursion (`Kernel`) for `par`:
- carry trie nodes **by value** (`{mask, first}`, 8 bytes), so `allowed = row.mask & col.mask` needs no load and only
  the two child steps touch memory;
- track `(r, c)` directly: no `cell / cols` and `cell % cols` (a real `div`, since `cols` isn't a constant);
- the row state is a single node, because finished rows are never needed again;
- the node counter is a local, not `self.nodes`.

Also tried `get_unchecked` in the child step: only 2–3 % (noise level), so I dropped it to keep the code free of `unsafe`.

| Benchmark | Before (parallel) | After (kernel) | Change |
|---|---|---|---|
| B1 count 3×3 | 7.8 ms | 6.4 ms | 1.23× |
| B2 find 5×6 | 0.73 ms | 0.59 ms | 1.25× |
| B3 find 6×6 | 1.51 ms | 1.23 ms | 1.23× |
| B4 count 4×4 | 1.48 s | 1.12 s | 1.33× |
| B5 count 4×5 | 6.43 s | 4.84 s | 1.33× |
| B6 count 3×7 | 0.63 s | 0.48 s | 1.30× |
| B7 find 7×7 | 0.70 s | 0.46 s | 1.51× |
| B8 find 6×7 | 0.63 s | 0.39 s | 1.59× |
| B9 find 5×9 | 1.06 s | 0.86 s | 1.23× |
| heavy total | 10.9 s | 8.2 s | **1.34×** |

**Verdict:** kept. Same node counts and identical grids (checked against `trie` on B7–B9).

### 2026-11-07: huge suite B10–B13 added

The heavy suite was down to ~8 s, and single runs of 0.4–1 s are noisy, so I added a tier sized at
"by-value node kernel" (~50 s total). Old IDs are unchanged.

| ID | Task | Baseline time | Result |
|---|---|---|---|
| B10 | count 5×5 | 30.6 s | 80,364,530 solutions (30.7 G nodes) |
| B11 | count 4×6 | 9.50 s | 37,812,638 solutions |
| B12 | find 6×8 | 5.10 s | acidemia/ninepins/unslings/reticule/imamates/careless |
| B13 | find 5×10 | 4.94 s | **no 5×10 exists** (3.8 G nodes, exhaustive) |

Probe: find 7×8 did not finish in 150 s; a future target.

### 2026-11-07: Level 7 — last-row count kernel (`04bbb6c`) ✅ (small)

**Idea:** in the last row, the column children are never used, only their masks. So copy the column masks into
a fixed array once per last-row entry and skip all column steps. Unroll the second-to-last cell as
`Σ popcount(step(row, l).mask & last_mask)`. Node counts are unchanged.

Instrumented 4×4 (temporary): rows 1/2/3 = 1.9 M / 244 M / 779 M nodes. **60 % of nodes are in the last row**,
so 4-row counts gain most. 5×5 has its weight in the middle rows and gains nothing.

| Benchmark | Before (kernel) | After (last-row) | Change |
|---|---|---|---|
| B1 count 3×3 | 6.4 ms | 5.9 ms | 1.07× |
| B4 count 4×4 | 1.12 s | 0.92 s | 1.21× |
| B5 count 4×5 | 4.84 s | 4.30 s | 1.13× |
| B6 count 3×7 | 0.48 s | 0.40 s | 1.22× |
| B10 count 5×5 | 30.6 s | 30.9 s | ≈ |
| B11 count 4×6 | 9.50 s | 8.96 s | 1.06× |
| B7–B9, B12–B13 find | — | — | ≈ (unchanged code; B8 0.39 → 0.56 s is parallel-find run-to-run noise) |

**Verdict:** kept. Note for the future: parallel `find` timings vary ±30 % between runs, because which job
wins and how much of later jobs is explored depends on thread timing. Treat single-run find gains below ~1.3× as noise.

### 2026-11-07: Level 5 — transpose symmetry for square counts (`43cfaf9`) ✅

**Idea:** in an R×R square whose rows and columns use the same word list, the transpose of a solution is a
solution, and transposing swaps (first row, first column). So

  count = 2 · #{row₀ < col₀} + #{row₀ = col₀}

`Kernel::count_sym` runs only on the "tied" path, where column 0 equals row 0 so far. At each (r, 0) it cuts
letters < row₀[r]. The equal letter stays tied. Greater letters drop into the unchanged hot kernel with
weight ×2. Job prefixes are capped to row 0, because they are expanded without the tie rule. Rectangles
(R ≠ C) are unaffected. Exact counts are unchanged: 1,909,732 / 136,245,247 / 80,364,530. Random-dictionary
2×2 and 3×3 cases are cross-checked against naive.

| Benchmark | Before (last-row) | After (symmetry) | Nodes | Change |
|---|---|---|---|---|
| B1 count 3×3 | 5.9 ms | 3.5 ms | 2.12 M → 1.10 M | 1.68× |
| B4 count 4×4 | 0.92 s | 0.47 s | 1.30 G → 0.66 G | **1.96×** |
| B10 count 5×5 | 30.9 s | 15.9 s | 30.7 G → 15.5 G | **1.94×** |
| rectangles / finds | — | — | — | ≈ (unchanged) |

**Verdict:** kept.

### 2026-11-07: Level 4 — row look-ahead / forward checking (`e5a081e`) ✅

**Idea:** a letter at (r, c) is only useful if the row node it leads to can still match the
columns further right. Each trie node now carries two look-ahead masks: `next` = union of the
children's masks (letters some completion has one position further), and `next2` = the same two
positions further. After stepping the row we already hold the child node, so the check is two ANDs
against `colv[c+2].mask` and `colv[c+3].mask`, with no extra memory load. It is in `count`,
`count_last` and `search`.

Tried on the way:
- a full `reach[node][p]` table over all positions: B5 nodes 5.05 G → 3.35 G, but a separate
  load plus a loop per node made it slower overall, time flat;
- the single `reach[c+2]` from the separate table: 4×6 1.19×;
- 12-byte nodes with `next` only: 4×6 1.22×;
- 16-byte nodes with `next` and `next2`: the version kept.

Bigger nodes cost the 4×4 square (short rows: few checks to gain) ~6%.

| Benchmark | Before (symmetry) | After (look-ahead) | Nodes | Change |
|---|---|---|---|---|
| B1 count 3×3 | 3.54 ms | 3.45 ms | 1.10 M → 1.05 M | ≈ |
| B3 find 6×6 | 1.22 ms | 0.72 ms | 49.8 k → 26.0 k | 1.70× |
| B4 count 4×4 | 0.471 s | 0.520 s | 0.66 G → 0.56 G | 0.91× (bigger nodes) |
| B5 count 4×5 | 4.32 s | 4.21 s | 5.05 G → 3.36 G | 1.03× |
| B6 count 3×7 | 0.398 s | 0.343 s | 485 M → 183 M | 1.16× |
| B7 find 7×7 | 0.608 s | 0.439 s | 159 M → 55 M | 1.38× |
| B8 find 6×7 | 0.552 s | 0.413 s | 121 M → 46 M | 1.34× |
| B9 find 5×9 | 0.875 s | 0.748 s | 608 M → 238 M | 1.17× |
| B10 count 5×5 | 15.95 s | 14.43 s | 15.5 G → 9.5 G | 1.11× |
| B11 count 4×6 | 9.12 s | 7.64 s | 9.59 G → 4.95 G | 1.19× |
| B12 find 6×8 | 5.24 s | 3.87 s | 4.13 G → 1.51 G | 1.35× |
| B13 no 5×10 | 5.04 s | 3.68 s | 3.80 G → 1.43 G | 1.37× |

**Verdict:** kept. Node counts drop 1.5–2.9×, but nodes/sec drops too (more work per node and
bigger nodes), so the wall-clock gain is 1.1–1.4×. Next: make the per-node work cheaper.
