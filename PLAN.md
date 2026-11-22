# Plan: things to figure out next

Working notes for the next optimization sessions. Every item that gets tried becomes a
benchmarked step (see CLAUDE.md). Failures too, labelled `(failed)`. Details of past steps:
[`results/journal.md`](results/journal.md). Main machine: **hephaestus** (Ryzen 9 3950X, 32 threads).

## Where we are (2026-11-22, after "mirror-image job test in find")

| | B5 count 4×5 | B10 count 5×5 | B15 count 5×6 | B18 count 6×6 | B7 find 7×7 | B13 prove no 5×10 | B16 prove no 6×9 |
|---|---:|---:|---:|---:|---:|---:|---:|
| start of the night ("unchecked trie steps") | 1.20 s | 3.91 s | 13.47 s | 20.9 s | 0.134 s | 0.99 s | 9.93 s |
| now | **0.728 s** | 3.91 s | **9.99 s** | **17.8 s** | **0.103 s** | **0.546 s** | **6.01 s** |

Where the time goes now (`results/charts/heatmap.svg`, `scripts/heatmap.py`):
- **counts:** the second-to-last row, its first 3 cells: 50–75% of nodes;
- **prove none, wide rows (6×9):** two rows above the last, columns 1–3. No next-row set there
  (9-letter rows overflow it).

## Learned tonight (details in the journal)

- **Exact next-row set** (up to 32 trie nodes, built lazily) is the big pruning win for counts,
  but only on rows ≤ 8 letters. On 9–10 letter rows it overflows and costs more than it prunes.
  A DAWG multiset version halves 6×9's nodes but costs about 2.7× per node (2.3× with an O(1) stamp merge).
- **Orientation matters a lot:** rows filled from their last letter (mirror image) are faster on
  every shape (1.07–1.7×); columns reversed are always slower. Counts use the mirror; squares ≥ 6×6
  too, with the **anti-diagonal symmetry** of the mirror image; big finds test their jobs on the
  mirror (job prefix forced at the end of row 0 via **virtual column nodes**).
- **Memory layout:** a 12-byte row node *without* `next2` was ~6% cheaper per node; packing the
  index into spare bits to keep `next2` cost more (decode on the critical path).
- **Code layout effects are real:** a merely *reachable* call to the specialized row kernel slowed
  the generic kernel 2–3% on 5-row grids. Specialize with const generics (compile-time instances).
- **Constants age:** the 2 M-node sequential find budget (from the slow days) cost 36 ms per find.
  Re-sweep old tuning constants after big speedups (done: S_CAP, jobs per thread, find budget).

## Ideas, roughly by expected value

1. **Mirror job tests for small grids too.** Today only big grids (area ≥ 48) use them, because their
   jobs must stay in row 0 and coarse jobs hurt finds with early solutions (5×8, 5×9). A dynamic
   scheme could start with fine jobs and switch to row-0 mirror jobs once the mirror is built and
   no solution has shown up, skipping the row-0 jobs already fully covered (both job lists are
   in lexicographic order). Determinism needs care: every fine job below a found solution must finish.
2. **A cheaper next-row set for wide rows.** The DAWG multiset prunes 2× on 6×9; it needs a
   representation where a child step costs about one trie step (e.g. a DAWG laid out so children are
   contiguous where possible, or sets as sorted id ranges).
3. **12-byte row node, `next2` dropped only where it doesn't prune** (it helped 4×6/5×6, not
   4×5/5×5). A per-width const choice, like `TRACK`.
4. **The second-to-last row on 5-row grids** (`count_pen` lost 3% there, reason unknown). Try
   again with a profiler if one ever becomes available (no perf on hephaestus: perf_event_paranoid=3, no root).
5. **Satisfiable find:** the job holding the first solution is walked twice (count test, then
   search); 7×7 pays ~10%. Have the `ANY` count record the grid of its first solution.
   Also: the specialized second-to-last-row kernel still loses on 5-row grids after the mirror (5×5 3.91 → 4.06 s).
6. **Bigger benchmarks:** B5/B6/B7/B8 are now under 0.2 s. Candidates (measured tonight):
   count 4×7 (1.7 s), count 5×7 (9.9 s mirrored), count 6×6 (20 s), count 7×7 (42 s, 11,669 solutions),
   prove no 7×8 (60 s), count 8×8 (28 s, 5 solutions, all symmetric squares).

## Done tonight

Exact next-row set ✅ · find kernel set ❌ · column look-ahead on the set ❌ · reuse look-ahead
children ❌ · 12-byte packed nodes ❌ · count-kernel job test in find ✅ · wide-row set ❌ ·
second-to-last-row kernel + uninit buffers ✅ · set cap 32 ✅ · short find budget ✅ · DAWG set ❌ ·
mirrored rows for counts ✅ · anti-diagonal symmetry ✅ · skip-gram bound ❌ · mirror-image job
test in find ✅. Plus: B17–B18 (replayed for every step), whole history replayed on hephaestus, B15–B16, the atlas
(every size to 10×22 settled; largest = 8×8), gallery, anatomy, heatmap and scaling charts.

## From the rust-skills checklist (still open)

| Rule | Status / idea |
|---|---|
| `opt-inline-never-cold` | mark job setup / prefix replay / symmetry path `#[cold]`; re-measure `inline(always)` |
| `lint-*` | `cargo clippy -- -W clippy::perf -W clippy::pedantic` once, fix what's cheap |
| `unsafe-miri-ci` | run `cargo +nightly miri test` on a tiny dict once (there is `unsafe` in the kernel) |
| `opt-pgo-profile` | retry PGO: the kernel changed a lot since the first try |
