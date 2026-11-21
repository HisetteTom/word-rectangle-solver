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

### 2026-11-07: compact column trie (`86cf07a`) ✅

**Idea:** the look-ahead masks made every node 16 bytes, but column nodes only need `mask` and
`first`. Cache math: the 4-letter trie (6,067 nodes) is 47 KiB at 8 B, which fits the 48 KiB L1D,
but 94 KiB at 16 B. The column trie now uses `CTrie` (8-byte `CNode`, same indices); only the
row trie keeps `next` and `next2`. Node counts are unchanged.

| Benchmark | Before (look-ahead) | After | Change |
|---|---|---|---|
| B4 count 4×4 | 0.520 s | 0.508 s | 1.02× |
| B5 count 4×5 | 4.21 s | 4.14 s | 1.02× |
| B8 find 6×7 | 0.413 s | 0.354 s | 1.17× (find noise ±) |
| B9 find 5×9 | 0.748 s | 0.691 s | 1.08× |
| B10 count 5×5 | 14.43 s | 13.83 s | 1.04× |
| B11 count 4×6 | 7.64 s | 7.43 s | 1.03× |
| B12 find 6×8 | 3.87 s | 3.74 s | 1.04× |
| B13 no 5×10 | 3.68 s | 3.53 s | 1.04× |

**Verdict:** kept: small but consistent. B4 is still behind its symmetry-step 0.471 s, because the
4-letter row trie is still 94 KiB.

### 2026-11-07: const-generic row width (`1f12ee3`) ✅

**Idea:** monomorphize the kernels per row width: `Kernel<'t, const W>`, dispatched once per job by
a `by_width!` macro (1..=32). With W known at compile time:
- `c + 2 < W` and `c == W - 1` compare against constants;
- column state is `[CNode; W]` and last-row masks are `[u32; W]` (no Vec, no 32-slot copy);
- most bounds checks disappear.

Node counts are unchanged.

| Benchmark | Before (compact col trie) | After | Change |
|---|---|---|---|
| B4 count 4×4 | 0.508 s | 0.486 s | 1.05× |
| B5 count 4×5 | 4.14 s | 3.89 s | 1.06× |
| B7 find 7×7 | 0.413 s | 0.370 s | 1.12× |
| B8 find 6×7 | 0.354 s | 0.282 s | 1.26× |
| B9 find 5×9 | 0.691 s | 0.608 s | 1.14× |
| B10 count 5×5 | 13.83 s | 12.49 s | 1.11× |
| B11 count 4×6 | 7.43 s | 7.01 s | 1.06× |
| B12 find 6×8 | 3.74 s | 3.41 s | 1.10× |
| B13 no 5×10 | 3.53 s | 3.19 s | 1.11× |

**Verdict:** kept. Every benchmark improves (5–14%, B8 more).

### 2026-11-07: throttled stop check in `search` (`d8a5b29`) ✅

**Idea:** `search` loaded the shared `best` atomic (through `stop()`) and tested the node budget at
every node. Now both are checked only when `nodes % 256 == 0`. Results stay deterministic: the
lowest job index still wins, and a job only aborts a little later. Counts don't use `search`.

| Benchmark | Before (const-generic) | After | Change |
|---|---|---|---|
| B7 find 7×7 | 0.370 s | 0.313 s | 1.18× (noisy) |
| B9 find 5×9 | 0.608 s | 0.606 s | ≈ |
| B12 find 6×8 | 3.41 s | 3.18 s | 1.07× |
| B13 no 5×10 | 3.19 s | 2.96 s | **1.08×** |
| counts | — | — | ≈ (untouched) |

**Verdict:** kept. B13, the cleanest find measurement (exhaustive, no early exit), gains 8%.

### 2026-11-07: branch-free look-ahead filter (`c8af9d0`) ✅

**Profile first:** `perf stat` on count 4×6 showed ~9.7 G branch misses for ~5 G nodes, about 2 per
node. One of them was the look-ahead `if … { continue }`: data-dependent, so mispredicted often.

**Idea:** `Kernel::good(row, allowed, …)` runs one straight-line pass over the allowed letters and
builds a "good letters" mask. A letter is good when its row child has `mask & col[c+1]`,
`next & col[c+2]` and `next2 & col[c+3]` all nonzero; it uses `&`/`|` on bools and a shift,
with no branches. The recursion loop then runs only over good letters, re-stepping the child, which
is now an L1 hit. The `col[c+1]` test is new: a dead child is now skipped before the call instead
of inside it, so node counts drop 20–30% (counts and solutions unchanged). It is used in `count`,
`count_last` (general cells) and `search`.

| Benchmark | Before (throttled stop) | After | Change |
|---|---|---|---|
| B4 count 4×4 | 0.488 s | 0.454 s | 1.07× |
| B5 count 4×5 | 3.89 s | 3.51 s | 1.11× |
| B6 count 3×7 | 0.324 s | 0.254 s | **1.27×** |
| B9 find 5×9 | 0.606 s | 0.506 s | 1.20× |
| B10 count 5×5 | 12.62 s | 11.42 s | 1.10× |
| B11 count 4×6 | 6.94 s | 6.21 s | 1.12× |
| B12 find 6×8 | 3.18 s | 2.51 s | **1.27×** |
| B13 no 5×10 | 2.96 s | 2.66 s | 1.11× |

**Verdict:** kept. 7–27% everywhere.

### 2026-11-07: iterative DFS for the counting kernel (`e25590c`) ❌ reverted

**Idea:** the profile split cycles 55% `count` (upper rows) and 45% `count_last`. I replaced the
per-cell recursion of `count` with an explicit-stack loop (per-cell `pend` / row node / column node
arrays, `'enter` + backtrack loop) to avoid call overhead and register spills.

| Benchmark | Before (branch-free look-ahead) | After | Change |
|---|---|---|---|
| B4 count 4×4 | 0.454 s | 0.479 s | 0.95× |
| B5 count 4×5 | 3.51 s | 3.64 s | 0.96× |
| B10 count 5×5 | 11.42 s | 12.96 s | 0.88× |
| B11 count 4×6 | 6.21 s | 6.45 s | 0.96× |
| finds | — | — | ≈ (untouched; B8 0.58× is noise) |

**Verdict:** reverted. The recursive version keeps `row`, `col` and `allowed` in registers, and
LLVM's calls are cheap. The explicit stacks add loads and stores and a harder-to-predict control flow.

### 2026-11-07: PGO (profile-guided optimization) — tried, no gain (no code change, no bench bar)

**Idea:** the kernel is branchy with data-dependent trip counts, so let LLVM lay out code from real
profiles. Steps: `-Cprofile-generate` build, trained on count 3×3/4×4/3×7/4×5 and find
6×6/7×7/5×9/6×8, then `llvm-profdata merge` and a `-Cprofile-use` build (rustc and llvm-profdata are both LLVM 23).

| Run (release LTO vs release + PGO) | release | PGO |
|---|---|---|
| count 4×5 | 3.52–3.62 s | 3.42–3.51 s |
| count 4×6 | 6.10–6.15 s | 6.03–6.32 s |
| count 4×4 | 0.487–0.499 s | 0.487–0.510 s |
| find 5×10 (exhaustive) | 2.71–2.87 s | 2.75–2.99 s |
| find 6×8 | 2.60–2.83 s | 2.62–2.71 s |

**Verdict:** dropped, within noise. Also, the instrumented binary took 5½ minutes to train, because
22 threads contend on shared profile counters. The hot kernels are tiny and already well laid out,
and release (LTO) ≈ fast here too.

### 2026-11-07: last-row memo on column-mask tuples — measured, not built

The last row's count depends only on the tuple of column masks, and different column nodes can
share a mask, so this could hit more often than the earlier node-tuple memo. I measured it with a
throwaway probe (calls at last-row start vs distinct mask tuples per thread):

| Instance | last-row calls | distinct | best possible hit rate |
|---|---|---|---|
| 3×3 | 52 k | 48 k | 7.6% |
| 3×7 | 6.1 M | 6.05 M | 1.0% |
| 4×5 | 350 M | 289 M | 17% |

**Verdict:** not worth a hash table, even unbounded. Mask tuples hardly repeat.

### 2026-11-07: depth-first trie layout (`2c1dbf2`) ❌ reverted

**Idea:** children must be contiguous (child = first + rank), but sibling groups can be in any order.
Lay them out in DFS pre-order instead of BFS, so each subtree occupies one region and search paths
stay local.

| Benchmark | Before (branch-free look-ahead) | After | Change |
|---|---|---|---|
| B4 count 4×4 | 0.454 s | 0.480 s | 0.95× |
| B5 count 4×5 | 3.51 s | 3.63 s | 0.97× |
| B10 count 5×5 | 11.42 s | 12.05 s | 0.95× |
| B11 count 4×6 | 6.21 s | 6.59 s | 0.94× |
| B12 find 6×8 | 2.51 s | 2.58 s | 0.97× |
| B13 no 5×10 | 2.66 s | 2.66 s | ≈ |

**Verdict:** reverted. The tries fit in L2 (at most ~270 KiB row + ~135 KiB column), and BFS
keeps the hot top levels packed together at the start of the array, which matters more.

### 2026-11-07: next-row forward check (`022d173`) ✅ kept

**Idea:** when cell (r, c) is placed, row r + 1 will need a letter at column c that (a) extends the
column word and (b) can follow a letter that row r + 1 can have at c − 1. Per-position **bigram
tables** (for each position k, letter set of position k → letters that can follow at k + 1 in some
row word) turn (b) into a mask with four 128-entry lookups: `uni(k, f)`. Carry `f` (letters row
r + 1 can still have at column c) along the row. A letter whose `uni(c−1, f) & column-child.mask`
is empty gets cut right away, instead of failing one full row later. It is applied in both the
count and the search kernels.

| Benchmark | Before (branch-free look-ahead) | After | Change |
|---|---|---|---|
| B4 count 4×4 | 0.454 s · 434 M nodes | 0.452 s · 397 M | 1.00× |
| B5 count 4×5 | 3.51 s · 2.44 G | 3.36 s · 2.14 G | 1.04× |
| B6 count 3×7 | 0.254 s | 0.250 s | 1.02× |
| B7 find 7×7 | 0.291 s | 0.286 s | 1.02× |
| B8 find 6×7 | 0.273 s | 0.256 s | 1.07× |
| B10 count 5×5 | 11.42 s · 6.88 G | 10.44 s · 5.39 G | **1.09×** |
| B11 count 4×6 | 6.21 s · 3.71 G | 5.81 s · 3.24 G | **1.07×** |
| B12 find 6×8 | 2.51 s · 1.16 G | 2.49 s · 1.01 G | 1.01× |
| B13 no 5×10 | 2.66 s · 1.14 G | 2.53 s · 0.98 G | 1.05× |

**Verdict:** kept. It cuts 9–22% of the nodes, but each node costs more (four extra table
loads), so the time gain is smaller than the node gain. It helps most where rows are many and
short (5×5).

### 2026-11-07: two-rows-ahead check (`bd0c975`) ❌ reverted

**Idea:** extend the forward check to row r + 2: column nodes get `next` (letters two rows down,
CNode 8 → 12 B), and a second forward mask `g` runs through the bigram tables.

| Benchmark | Before (next-row forward check) | After | Change |
|---|---|---|---|
| B4 count 4×4 | 0.452 s · 397.3 M nodes | 0.443 s · 397.3 M | ≈ |
| B5 count 4×5 | 3.36 s · 2.143 G | 3.40 s · 2.142 G | 0.99× |
| B10 count 5×5 | 10.44 s · 5.39 G | 10.81 s · 5.15 G | 0.97× |
| B11 count 4×6 | 5.81 s | 5.93 s | 0.98× |
| B12 find 6×8 | 2.49 s | 2.56 s | 0.97× |
| B13 no 5×10 | 2.53 s | 2.49 s | 1.02× |

**Verdict:** reverted. When row r + 2 exists, the next-row check plus `good()` already catch
nearly every dead end. It prunes 0.03% of nodes on 4-row grids and 4.5% on 5×5, which doesn't
pay for the bigger column nodes and the extra lookups.

### 2026-11-07: branch-free forward check (`1a4c1f0`) ❌ reverted

**Idea:** the next-row check (`base & column child mask == 0 → skip`) is a data-dependent branch
right after the branch-free `good()` pass. Fold it into that pass (`good_fw`) so the second loop
only visits survivors. A second version also pre-filtered the end-of-row cell the same way.

| Benchmark | Before (next-row forward check) | After | Change |
|---|---|---|---|
| B4 count 4×4 | 0.452 s | 0.434 s | 1.04× |
| B5 count 4×5 | 3.36 s | 3.25 s | 1.03× |
| B10 count 5×5 | 10.44 s | 10.31 s | 1.01× |
| B11 count 4×6 | 5.81 s | 5.85 s | 0.99× |

With 3 interleaved A/B runs it looked flat on 4×5 and 5×5 and 3% slower on 4×6. The end-of-row
pre-filter was 3–4% slower everywhere and wasn't kept even in the experiment.

**Verdict:** reverted as noise. Few letters survive `good()`, so the extra column step for every
candidate costs about as much as the mispredictions it removes.

### 2026-11-07: tall grids transposed (`e9dd7bb`) ✅ kept, new benchmark B14

**Idea:** an R×C rectangle transposed is a C×R rectangle with the word lists swapped, so both
have the same count. Row-major search is much cheaper with **few long rows**: long rows give the
row look-ahead (`next`, `next2`) and the bigram forward check more to prune, and the job split
stays in row 0. Measured at "next-row forward check":

| Shape | nodes | time | Transposed | nodes | time |
|---|---|---|---|---|---|
| 6×4 | 7.69 G | 14.1 s | 4×6 | 3.24 G | 5.9 s |
| 5×4 | 3.16 G | 5.15 s | 4×5 | 2.14 G | 3.21 s |
| 7×3 | 484 M | 0.83 s | 3×7 | 133 M | 0.25 s |
| 8×6 find | 5.08 G | 11.6 s | 6×8 find | 1.01 G | 2.5 s |

So `count` on a tall grid (rows > cols) now runs the transposed search. `find` keeps its
orientation, because its answer must be the row-major lexicographically first grid (tests pin it
to naive's answer). New benchmark **B14 = count 6×4**, with a baseline bar recorded first.

| Benchmark | Before | After | Change |
|---|---|---|---|
| B14 count 6×4 | 14.29 s · 7.69 G nodes | 5.69 s · 3.24 G | **2.51×** |
| others | | | unchanged (noise) |

**Verdict:** kept.

### 2026-11-07: paired last two rows (`864443e`) ❌ reverted

**Measurement first.** A throwaway probe counted, per row, the count-kernel calls whose subtree
holds zero solutions:

| Shape | row 1 dead | row 2 dead | row 3 dead |
|---|---|---|---|
| 4×5 | 36% | **79%** (second-to-last) | — |
| 4×6 | 64% | **94%** (second-to-last) | — |
| 5×5 | 52% | 94% | **97%** (second-to-last, 3.27 G nodes) |

Even at the *last* column of the second-to-last row, 92% of calls are dead. The last row only
appears through the approximate bigram check until the row above it is complete.

**Idea:** fill the last two rows together, column by column (letter `a` of row R−2, then letter `b`
of row R−1, which must end the column). Both rows walk their trie, so an impossible last row
is seen at the first column where it fails. The last row also gets `good()` look-ahead against
the letters each column can still end with.

| Benchmark | Before (tall grids transposed) | After | Change |
|---|---|---|---|
| B4 count 4×4 | 0.437 s · 397 M nodes | 0.426 s · 221 M | 1.03× |
| B5 count 4×5 | 3.26 s · 2.14 G | 3.51 s · 0.95 G | 0.93× |
| B6 count 3×7 | 0.247 s · 133 M | 0.732 s · 238 M | 0.34× |
| B10 count 5×5 | 10.30 s · 5.39 G | 12.73 s · 4.07 G | 0.81× |
| B11 count 4×6 | 5.69 s · 3.24 G | 6.96 s · 1.63 G | 0.82× |

**Verdict:** reverted. Nodes drop by up to 2.2×, but each node branches on two letters and runs
two filters, so it costs 2–2.4× more, and the old last-row kernel counted its leaves very
cheaply. On 3×7 the row-at-a-time order with the bigram check prunes much better.
The dead-end numbers stay useful: the second-to-last row is where the waste is.

### 2026-11-07: backward next-row bound (`7901ad2`) ❌ reverted

**Idea:** a cheap stand-in for the paired rows. At every row start, bound row r + 1 from the
right: `bk[k]` = letters column k could give it (union over the letters row r might put
there), narrowed by backward bigram tables so that a suffix k..W is still possible. Then each
node pays one extra AND in the forward check, and an empty `bk[k]` kills the whole row at once.

| Benchmark | Before (tall grids transposed) | After | Change |
|---|---|---|---|
| B4 count 4×4 | 0.437 s · 397.3 M nodes | 0.437 s · 396.8 M | ≈ |
| B5 count 4×5 | 3.26 s · 2.143 G | 3.30 s · 2.140 G | 0.99× |
| B10 count 5×5 | 10.30 s · 5.39 G | 11.73 s · 5.04 G | 0.88× |
| B11 count 4×6 | 5.69 s · 3.244 G | 5.88 s · 3.240 G | 0.97× |

**Verdict:** reverted. Before row r is placed, a column allows almost any letter one row down,
so the bound is close to "all letters": 0.1% fewer nodes on 4-row grids, 6.5% on 5×5. The
per-row-start work (one column-child step per letter, W table lookups) costs more than that.
The pruning that matters has to depend on the letters actually placed in row r. The forward
check already does that.

### 2026-11-07: thread scaling measured (no code change, no bar)

Count 5×5 (B10 task) pinned to CPU subsets with `taskset`. `available_parallelism` follows the
affinity mask, so the thread count matches. Core Ultra 9 185H: 6 P-cores with 2 hyperthreads each
(CPUs 0–11), 8 E-cores (12–19), 2 low-power E-cores (20–21).

| CPUs | threads | time |
|---|---|---|
| all (0–21) | 22 | **10.34 s** |
| no LP-E (0–19) | 20 | 11.23 s |
| P-cores only, with HT (0–11) | 12 | 17.64 s |
| 1 thread per P-core + 8 E (0,1,3,6,8,10,12–19) | 14 | 12.75 s |
| E-cores only (12–19) | 8 | 26.26 s |

**Verdict:** every thread helps, even hyperthreads and the slow LP-E cores, because the kernel
is latency-bound (branch misses, dependent loads), not throughput-bound. The dynamic job queue
already balances P and E cores. No change.

### 2026-11-07: unchecked trie steps (rust-skills `opt-bounds-check`, `mem-assert-type-size`)

**Idea:** the asm of `Kernel<5>::count` showed, for every trie step, a load of the `Vec` length,
a compare and a branch to `panic_bounds_check`, plus an extra pointer hop (`&Trie` → `Vec` →
data). Same for `rt.node(0)` at every row start and for `bi[k]`. The kernel now holds plain
slices (`&[Node]`, `&[CNode]`), a `&[Bigram; W]` and a cached root node. Steps go through
`unsafe fn rstep / cstep` (`get_unchecked`) with a `// SAFETY:` contract: the letter is a bit of
the node's mask, so the child exists (trie builder invariant). A `debug_assert!` checks it in
the tests. A `const _: () = assert!(size_of::<Node>() == 16 && size_of::<CNode>() == 8)` guards
the layout.

| Benchmark | Before (tall grids transposed) | After | Change |
|---|---|---|---|
| B4 count 4×4 | 0.437 s | 0.402 s | 1.09× faster |
| B5 count 4×5 | 3.26 s | 3.07 s | 1.06× faster |
| B9 find 5×9 | 0.534 s | 0.465 s | 1.15× faster |
| B10 count 5×5 | 10.30 s | 9.81 s | 1.05× faster |
| B11 count 4×6 | 5.69 s | 5.49 s | 1.04× faster |
| B13 prove no 5×10 | 2.57 s | 2.40 s | 1.07× faster |
| B14 count 6×4 | 5.69 s | 5.55 s | 1.02× faster |

A/B before the bench (3 alternating runs each): 4×5 3.20–3.26 → 3.04–3.12 s, 5×5 10.26–10.31 →
9.62–9.65 s. The node counts are unchanged (same search, cheaper nodes).

**Verdict:** kept. A small, consistent win on every bench. This is the first rust-skills rule
that paid off directly.

### 2026-11-21: new main machine (hephaestus): whole history replayed, B15–B16 added

**Why:** the first machine's numbers are not comparable with anything run elsewhere. Spot checks
showed hephaestus (Ryzen 9 3950X, 16 cores / 32 threads) about **2.5× faster on every step**, and
2.7× even single-threaded, so mixing the two in one chart would show a fake jump.

**What:** `scripts/replay.py` rebuilt all 27 steps (kept and failed) from their own commits (`fast`
profile, today's compiler) and ran each one on the current suite: naive on B1–B3, the single-threaded
steps on B1–B9, every parallel step on B1–B16. That is 375 runs, about 50 min. Median of 5 runs under 0.5 s,
median of 3 under 5 s, one run above that. Every step gives the same answer on every benchmark
(solution counts, found / no solution). The old log moved to `results/legacy/bench-log.csv`.
`bench.sh` now refuses to log on another host. The step "parallel 22 threads" is now "parallel (all cores)".
The bench-only row "B14 baseline (tall count)" was dropped, because every step now has a B14 bar.

**New benchmarks** (the heavy suite is now 0.1–1.2 s and B1–B3 are under 2 ms):

| ID | Task | parallel (all cores) | unchecked trie steps |
|---|---|---:|---:|
| B15 | count all 5×6 (7,424,362 solutions) | 30.0 s | 13.5 s |
| B16 | prove no 6×9 exists | 26.0 s | 9.93 s |

(Prove no 7×8 exists took 60 s and was left out, to keep the replay and the full bench short. It is
the next candidate.)

**Did any verdict change?** Each step vs. the previous kept step, geometric mean over B4+:

| Step | Old machine | Hephaestus |
|---|---:|---:|
| parallel (all cores) | 12.3× | 11.3× (16 physical cores vs 22 threads) |
| by-value node kernel | 1.37× | 1.22× |
| last-row search (failed) | 0.96× | 1.02× (within noise: still not worth it) |
| branch-free forward check (failed) | 0.92× | 1.00× |
| iterative DFS (failed) | 0.91× | 0.99× |
| unchecked trie steps | 1.056× | 1.055× |

All the other steps agree within a few percent. No kept step turns into a loss and no failed experiment
becomes a clear win.

**Headline (current code vs. each suite's baseline, geometric mean):** 6,723× faster than naive
(B1–B3), 35.6× faster than trie + masks (B4–B9), 2.86× faster than the first parallel version
(B10–B16). The new `results/charts/progress.svg` shows the whole history on one line.

### 2026-11-21: exact next-row set in the count kernel (`a651f45`) ✅ kept

**Idea (PLAN idea 1):** the bigram check only approximates the next row by "which letters can
follow these letters". While filling row r, the kernel now tracks the **exact set S of row-trie
nodes that row r + 1 can still be in**, given the column letters already forced on it (each placed
letter fixes the mask of what its column allows one row down). That is up to `S_CAP` = 16 nodes,
kept in a stack array. When S grows past 16, the kernel falls back to bigrams for the rest of the row.
- prune: a letter dies when no node of S has a child the column allows (the OR of S's masks);
- in the **second-to-last row**, the last cell sums `popcount(s.mask & column mask)` over S: the
  number of last rows, with no last-row walk at all;
- **lazy:** a call gets (parent set, column mask) and builds S only if its cell has a live letter.
  Building it eagerly for every letter made 5×5 and 5×6 *slower* (4.30 s, 14.3 s), even though
  they visit 40–48% fewer nodes.

Cap sweep (lazy version, fast profile):

| S_CAP | 4×5 | 5×5 | 4×6 | 5×6 |
|---|---:|---:|---:|---:|
| 8 | 0.91 s | 3.87 s | 1.70 s | 13.0 s |
| **16** | 0.86 s | 3.93 s | 1.59 s | 12.4 s |
| 32 | 0.84 s | 4.08 s | 1.58 s | 12.3 s |
| 64 | 0.99 s | 4.89 s | 1.84 s | 14.1 s |

| Benchmark | Before (unchecked trie steps) | After | Change |
|---|---|---|---|
| B4 count 4×4 | 0.157 s · 397 M nodes | 0.132 s · 193 M | 1.19× faster |
| B5 count 4×5 | 1.20 s · 2.14 G | 0.864 s · 0.61 G | 1.39× faster |
| B10 count 5×5 | 3.91 s · 5.39 G | 3.95 s · 3.27 G | ≈ |
| B11 count 4×6 | 2.19 s · 3.24 G | 1.59 s · 1.05 G | 1.38× faster |
| B14 count 6×4 | 2.18 s | 1.59 s | 1.37× faster |
| B15 count 5×6 | 13.47 s · 16.3 G | 12.43 s · 8.50 G | 1.08× faster |

**Verdict:** kept. This is the first attack on the dead second-to-last row that paid off (after
three failed ones). Nodes drop 2–3.5×, and each node costs more. 4-row grids gain about 1.4×,
because there the tracked row is the last one. On 5-row grids the earlier rows pay the overhead
and get only part of the benefit.

### 2026-11-21: exact next-row set in the find kernel (`be5eb79`) ❌ reverted

**Idea:** give `search` (find, and the "prove none" runs B13/B16) the same lazy next-row set
as the count kernel, for pruning only (find must still walk the last row to write the grid).

| Benchmark | Before (exact next-row set) | After | Change |
|---|---|---|---|
| B7 find 7×7 | 0.129 s | 0.166 s | 0.78× |
| B9 find 5×9 | 0.194 s | 0.233 s | 0.83× |
| B12 find 6×8 | 1.03 s · 1.01 G nodes | 1.09 s · 0.69 G | 0.94× |
| B13 prove no 5×10 | 0.98 s · 0.98 G | 1.13 s · 0.60 G | 0.87× |
| B16 prove no 6×9 | 9.91 s · 10.5 G | 10.84 s · 6.99 G | 0.91× |

Tracking the set only for the last row (from the second-to-last row) is worse still: 6×9 took 12.0 s
with almost no fewer nodes, because on 8–10 letter rows the set overflows its 16 slots at once.

**Verdict:** reverted. The count kernel won because the set also *replaced* the last-row walk.
Here it only prunes, and 33–38% fewer nodes don't pay for building the set plus a 256-byte
buffer in every frame.

### 2026-11-21: column look-ahead on the next-row set (`ab2d8f3`) ❌ reverted

**Idea:** column-trie nodes get a `next` mask (the letters the column allows one row further
down). While building the next-row set, drop every prefix whose letters at c, c+1, c+2 (`mask`,
`next`, `next2` of the row node) can't fit what columns c, c+1, c+2 allow one row down. If the
set becomes empty, the cell dies before trying any letter.

| Benchmark | Before (exact next-row set) | After | Change |
|---|---|---|---|
| B5 count 4×5 | 0.864 s · 614 M nodes | 0.894 s · 521 M | 0.97× |
| B10 count 5×5 | 3.95 s · 3.27 G | 4.16 s · 3.13 G | 0.95× |
| B11 count 4×6 | 1.59 s · 1.05 G | 1.64 s · 0.88 G | 0.97× |
| B15 count 5×6 | 12.43 s · 8.50 G | 12.95 s · 7.50 G | 0.96× |
| B16 prove no 6×9 | 9.91 s | 10.35 s | 0.96× |

The 12-byte column node alone (without the filter) costs 2–3% everywhere. The filter cuts nodes by
4–17% but costs about as much as it saves.

**Verdict:** reverted. The column's "one row down" mask is a union over every letter the current
row might still put there, which is too loose to prune much. Same lesson as "backward next-row bound".

Also in this round: the next-row set is now tracked only on rows of **≤ 8 letters**
(compile-time per width). On 9-letter rows it overflows its 16 slots at once (count 6×9: 10.5 s
with it, 9.4 s without), while 8-letter rows still gain (count 4×8: 2.38 → 2.10 s). No current
benchmark counts rows that wide, so this has no bar of its own.

### 2026-11-21: reuse the look-ahead's row children (`009f4fe`) ❌ reverted

**Idea (PLAN idea 3):** `good()` already steps into the row child of every allowed letter. Keep those
children in a 26-slot `MaybeUninit` stack buffer (branch-free compaction), so the main loop and
the last-row walk don't step the trie a second time.

| Benchmark | Before (exact next-row set) | After | Change |
|---|---|---|---|
| B5 count 4×5 | 0.864 s | 0.872 s | ≈ |
| B10 count 5×5 | 3.95 s | 4.02 s | 0.98× |
| B11 count 4×6 | 1.59 s | 1.65 s | 0.96× |
| B15 count 5×6 | 12.43 s | 12.86 s | 0.97× |

**Verdict:** reverted. The second step is an L1 hit (the child was loaded a few cycles earlier), so
it costs a popcount and a load. A store per letter into a 416-byte buffer, plus a counter, costs more.

### 2026-11-21: 12-byte packed row nodes (`d9ff337`) ❌ reverted

**Probe first:** dropping the `next2` look-ahead mask (12-byte nodes, weaker pruning) made 4×5 and 5×5
*faster* (0.852 s, 3.87 s) despite 3–4% more nodes, so the cost per node fell about 6%: memory footprint
matters. The 5-letter row trie (16-byte nodes) plus the column trie come to about 480 KB, against 512 KB of L2
per core, shared by 2 SMT threads.

**Idea:** keep all three masks and pack the 18-bit first-child index into their spare top 6 bits
(12 bytes per node, ≤ 2^18 nodes per trie). Users AND masks with clean 26-bit masks (`LETTERS`
replaces `!0` in the look-ahead), so the index bits never leak into letter sets.

| Benchmark | Before (exact next-row set) | After | Change |
|---|---|---|---|
| B5 count 4×5 | 0.864 s | 0.913 s | 0.95× |
| B10 count 5×5 | 3.95 s | 4.24 s | 0.93× |
| B11 count 4×6 | 1.59 s | 1.70 s | 0.94× |
| B15 count 5×6 | 12.43 s | 13.35 s | 0.93× |
| B16 prove no 6×9 | 9.91 s | 10.25 s | 0.97× |

**Verdict:** reverted. Decoding the index costs 5 shifts/ORs on the critical path of every child step
(load → decode → next load), and that costs more than the smaller footprint saves. A smaller node
must keep `first` directly addressable.

### 2026-11-21: count-kernel job test in find (`759d725`) ✅ kept (trade-off)

**Idea:** on exhaustive work the count kernel beats `search` (6×9: 9.4 s vs 9.9 s), and on rows of
≤ 8 letters it has the exact next-row set. So in `find`'s parallel phase each job is first *tested*
with the count kernel, and only a job with solutions is searched (from its start, so the answer is
still the lexicographically first grid: determinism unchanged). For this the count kernel gets a
compile-time `ANY` mode (monomorphized, so plain counts don't pay for it):
- return as soon as the total is non-zero;
- every 1024 nodes, check whether an earlier job already has a solution, and if so cancel (a sticky
  flag makes every frame return at once), as `search` does with `stop()`.

Without these two, 7×7 got almost 2× slower (0.24 s), because jobs holding solutions or coming after the
answer were counted to the end.

| Benchmark | Before (exact next-row set) | After | Change |
|---|---|---|---|
| B7 find 7×7 | 0.129 s | 0.144 s | 0.90× |
| B8 find 6×7 | 0.115 s | 0.120 s | 0.96× |
| B9 find 5×9 | 0.194 s | 0.188 s | 1.03× |
| B12 find 6×8 | 1.025 s | 1.032 s | ≈ |
| B13 prove no 5×10 | 0.976 s | 0.941 s | 1.04× |
| B16 prove no 6×9 | 9.91 s | 9.26 s | 1.07× |

**Verdict:** kept, as a trade-off. Exhaustive proofs, the long runs that settle the atlas frontier,
get 4–7% faster. A satisfiable find walks the job holding the answer twice (test, then search), which
costs about 10 ms on runs of about 0.13 s.

### 2026-11-22: next-row set on wide rows, second-to-last row only (`9618022`) ❌ reverted

**Idea:** on rows of 9+ letters the set overflows in the upper rows, where columns still allow many
letters. In the second-to-last row the columns are almost complete words, so I expected the set to stay
small there. So: track it only in that row when W > 8.

| Benchmark | Before (count-kernel job test) | After | Change |
|---|---|---|---|
| count 6×9 (not a benchmark) | 9.4 s · 10.49 G nodes | 11.1 s · 10.49 G | 0.85× |
| B13 prove no 5×10 | 0.94 s | 1.10 s | 0.85× |
| B16 prove no 6×9 | 9.26 s | 10.9 s | 0.85× |

**Verdict:** reverted. The node count barely moves: even in the second-to-last row, 9–10 letter rows
have more than 16 compatible next-row prefixes, so the set overflows and only adds cost.

### 2026-11-22: second-to-last-row kernel + uninitialized set buffers (`a3bbca3` ❌, `38ed419` ✅)

**Why:** the new heatmap (`charts/heatmap.svg`) shows the second-to-last row taking 60–70% of all
count nodes, and it ran through the generic kernel and all its branches.

**Idea:** `count_pen`, a kernel just for that row, where the next-row set (the possible last rows)
is always known:
- no bigram fallback and no row change;
- a set cap of 32 instead of 16;
- the **last two cells fused**: for letter l and last letter x, add up
  `popcount(child.mask & column_end(x))` over the set's children, with no call and no set buffer;
- if the set overflows, the generic kernel takes over.

Plus a separate fix found on the way: the set buffer was **zero-initialized at every node**
(`[Node::default(); 16]`, a 256-byte memset). It is now `MaybeUninit` (`SetBuf`).

The first version (`a3bbca3`, bar "second-to-last-row kernel (failed)") used `count_pen` everywhere:

| Shape | generic + uninit buffers | with count_pen | |
|---|---:|---:|---|
| 4×4 | 0.133 s | 0.115 s | 1.16× |
| 4×5 | 0.866 s | 0.773 s | 1.12× |
| 3×5 · 3×7 | 0.031 · 0.089 s | 0.027 · 0.086 s | 1.1× · 1.03× |
| 4×7 | 2.17 s | 2.23 s | 0.97× |
| 5×5 · 5×6 | 3.90 · 12.5 s | 4.01 · 12.8 s | 0.97× |
| 6×6 | 20.5 s | 21.4 s | 0.96× |

I couldn't isolate the 5+ row slowdown without a profiler (fusion on or off, `inline(never)`: no change).
It also showed up when 5-row grids never *called* `count_pen` (a runtime `rows <= 4` check): the call
site alone cost the generic kernel 2–3%. So the kept version (`38ed419`) picks it **at compile time**:
`count`/`count_sym` get a const `P`, and only grids of ≤ 4 rows and ≤ 6 columns run the `P = true`
instance. Everything else runs code identical to the generic kernel.

| Benchmark | Before (count-kernel job test) | After | Change |
|---|---|---|---|
| B4 count 4×4 | 0.132 s | 0.117 s | 1.13× faster |
| B5 count 4×5 | 0.872 s | 0.761 s | 1.15× faster |
| B10 count 5×5 | 3.99 s | 3.93 s | ≈ |
| B11 count 4×6 | 1.584 s | 1.582 s | ≈ |
| B15 count 5×6 | 12.41 s | 12.47 s | ≈ |

**Verdict:** kept: 13–15% on the 4-row counts, neutral elsewhere.

### 2026-11-22: next-row set cap 32 (`a182ecf`) ✅ kept (small)

With the set buffer no longer zero-filled, a bigger cap costs nothing until it is used. Sweep (median of 3):

| S_CAP | 5×5 | 5×6 | 4×7 | 4×5 |
|---|---:|---:|---:|---:|
| 16 | 3.93 s | 12.44 s | 2.19 s | 0.769 s |
| 24 | 3.93 s | 12.10 s | 2.15 s | 0.767 s |
| **32** | 3.95 s | 11.99 s | 2.16 s | 0.770 s |

Re-checked the width cutoff with cap 32: tracking 9–10 letter rows is still slower (count 6×9 9.65 →
10.37 s, 5×10 0.96 → 1.12 s), so it stays at ≤ 8 letters.

| Benchmark | Before | After | Change |
|---|---|---|---|
| B12 find 6×8 | 1.020 s | 0.947 s | 1.08× faster |
| B15 count 5×6 | 12.47 s | 12.00 s | 1.04× faster |
| others | | | within noise |

**Verdict:** kept.

### 2026-11-22: short sequential find budget (`ae085c4`) ✅ kept

**Probe:** find scaled badly: 7×7 took 0.63 s on 1 thread and 0.14 s on 32 (4.5×), and 16 threads beat 32. Timing
the phases showed `find`'s **sequential first attempt** (budget: 2 M nodes, meant to keep tiny cases
off the thread pool) costing **36 ms** before any thread started, on every find too big to finish there.
Job expansion took only 3–6 ms.

Budget sweep (median of 5):

| budget | 7×7 | 6×7 | 5×9 | 6×8 | 5×8 | 4×11 | 6×6 |
|---|---:|---:|---:|---:|---:|---:|---:|
| 2 M | 0.140 s | 0.115 s | 0.187 s | 0.948 s | 55 ms | 80 ms | 0.23 ms |
| 300 k | 0.109 s | 0.088 s | 0.156 s | 0.918 s | 26 ms | 49 ms | 0.23 ms |
| 100 k | 0.104 s | 0.083 s | 0.152 s | 0.911 s | 23 ms | 44 ms | 0.23 ms |
| **30 k** | 0.105 s | 0.081 s | 0.150 s | 0.913 s | 21 ms | 43 ms | 0.23 ms |
| 10 k | 0.104 s | 0.082 s | 0.150 s | 0.911 s | 21 ms | 42 ms | 6.9 ms (went parallel) |

| Benchmark | Before (next-row set cap 32) | After | Change |
|---|---|---|---|
| B7 find 7×7 | 0.139 s | 0.104 s | 1.34× faster |
| B8 find 6×7 | 0.120 s | 0.083 s | 1.45× faster |
| B9 find 5×9 | 0.186 s | 0.152 s | 1.22× faster |
| B12 find 6×8 | 0.947 s | 0.912 s | 1.04× faster |
| B13 prove no 5×10 | 0.936 s | 0.901 s | 1.04× faster |

**Verdict:** kept. A constant from the early days (sized when one node cost 5× more) had become the
biggest cost of the short finds. From this step on, heavy and huge benchmarks are the median of 3 runs.

### 2026-11-22: next-row set over a DAWG, with multiplicities (`631c1c3`) ❌ reverted

**Idea:** the next-row set holds one row-trie node per prefix, but two prefixes that can end the same
ways behave the same from here on. Over a minimized automaton (DAWG, restored from `ad3fb0c`), the set
becomes a multiset of states: a new child that lands on a state already in the set just adds its
multiplicity (`SetBuf::merge`). The second-to-last row adds `mult × popcount`, so counts stay exact.
I expected smaller sets, fewer overflows, and a way to finally track 9–10 letter rows.

| Shape | trie set | DAWG set | |
|---|---:|---:|---|
| count 5×5 | 3.93 s · 3.27 G nodes | 4.46 s · 3.12 G | 0.88× |
| count 5×6 | 12.0 s · 8.50 G | 15.6 s · 6.96 G | 0.77× |
| count 6×9, tracked (was off) | 9.4 s · 10.49 G | 13.5 s · **5.64 G** | 0.70× |
| count 6×9, second-to-last row only | 9.4 s | 10.7 s · 10.49 G | 0.88× |

**Verdict:** reverted. The merged set prunes hard: on 6×9 it halves the nodes, which the trie set
never managed. But each node costs about 2.7× more: two dependent loads per child step (edge, then node),
plus a linear merge scan over up to 32 entries on every insert. The node halving comes from the
upper rows (tracking only the second-to-last row gives nothing), and that's where the sets are biggest.
**Follow-up, same night: O(1) merge.** A per-thread stamp table (one `u64` per DAWG state:
`epoch << 8 | slot`) replaced the linear scan. Not committed: it's a variant of the same experiment.

| Shape | trie set | DAWG + scan | DAWG + stamp table |
|---|---:|---:|---:|
| count 4×5 | 0.77 s | 0.94 s | 0.84 s |
| count 5×5 | 3.93 s | 4.46 s | 4.24 s |
| count 5×6 | 12.0 s | 15.6 s | 13.2 s |
| count 6×9 (tracked) | 9.4 s (untracked) | 13.5 s | 11.4 s · 5.64 G nodes |
| count 5×10 (tracked) | 0.96 s (untracked) | — | 1.26 s · 0.54 G nodes |

Still slower everywhere: each child now costs an edge load, a node load and a stamp read and write.
The idea stays parked. It would need a set structure that costs about what one trie step costs.

### 2026-11-22: mirrored rows for counts (`ad20594`) ✅ kept

**Idea:** a count doesn't change if the grid is flipped. A horizontal mirror image has every row word
reversed (and the columns in reverse order, from the same list). A vertical one has the column words
reversed. So a count can run in any of 4 orientations. I measured them with dictionaries whose words of the
row length and/or column length are reversed (same counts every time, as they must be):

| Shape | as asked | rows reversed (mirror) | columns reversed | both (180°) |
|---|---:|---:|---:|---:|
| count 4×5 | 0.77 s | **0.72 s** | 0.94 s | 1.19 s |
| count 4×6 | 1.58 s | **1.40 s** | 2.26 s | 2.95 s |
| count 4×7 | 2.13 s | **1.71 s** | 3.34 s | 4.50 s |
| count 5×6 | 11.97 s | **9.99 s** | 20.5 s | 29.7 s |
| count 5×7 | 14.10 s | **9.88 s** | 26.0 s | 41.6 s |
| prove no 5×10 (count) | 0.96 s | **0.59 s** | 3.84 s | 8.81 s |
| prove no 6×9 (count) | 9.50 s | **6.88 s** | 12.05 s | 26.1 s |

Filling each row **from its last letter** wins every time, by 1.07–1.63×. Reversing the columns
loses every time. My guess: word endings (-s, -ed, -er, -ing) are more predictable than beginnings,
so starting a row from its end narrows the row trie faster, while the columns are better used
top-down, where their first letters are spread out.

So `count` now runs non-square grids on the mirrored search (built lazily, like the transposed one
for tall grids). Squares with one word list keep the transpose symmetry instead: mirrored without it, 5×5
takes 7.2 s against 4.0 s.

| Benchmark | Before (short sequential find budget) | After | Change |
|---|---|---|---|
| B5 count 4×5 | 0.771 s | 0.728 s | 1.06× faster |
| B11 count 4×6 | 1.581 s | 1.408 s | 1.12× faster |
| B14 count 6×4 | 1.586 s | 1.417 s | 1.12× faster |
| B15 count 5×6 | 11.99 s | 10.14 s | 1.18× faster |
| B6 count 3×7 | 85 ms | 89 ms | ≈ (73.7 M nodes vs 108 M; building the second search costs ~3 ms) |

**Verdict:** kept. `find` can't use it: its answer must be the first grid in the asked orientation.
"Prove none" runs (B13, B16) would gain 1.4–1.6× if they were counts. That's a reason to add a
dedicated "exists?" mode later.
