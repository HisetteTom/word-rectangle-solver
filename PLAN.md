# Plan: things to figure out next

Working notes for the next optimization sessions. Every item that gets tried becomes a
benchmarked step (see CLAUDE.md). Failures too, labelled `(failed)`. Details of past steps:
[`results/journal.md`](results/journal.md).

## Where the time goes now (at "tall grids transposed")

- Count kernel ≈ 84% of 5×5 cycles. About 170 instructions per node. Branch misses ≈ 16% of cycles,
  L1 misses negligible. IPC ≈ 1.8.
- **The waste is in the second-to-last row:** 94–97% of its nodes lead to zero solutions
  (5×5: 3.27 G nodes there, 97% dead; even its last column is 92% dead).
  Tried and failed so far: paired last two rows (fewer nodes, but each costs 2.4×), backward
  next-row bound (too loose), two-rows-ahead check (prunes nothing).

## Ideas, roughly by expected value

1. **Exact last-row prefix check, cheaply.** In the second-to-last row, keep the *set of last-row
   trie nodes* consistent with the column-end masks so far, but only while it is small (e.g. ≤ 8
   nodes, else fall back to the bigram check). The bigram check is only an approximation of this.
2. **Hybrid pairing:** switch to "both last rows together" only for the last k columns, where the
   dead ratio is highest and the pair branching is small.
3. **Cheaper nodes in the count kernel:**
   - `good()` already steps every row child, and the main loop steps it again. Keep the children
     from the `good()` pass in a small stack array.
   - Keep the last-row masks incrementally (write `cn.mask` into an array as columns are stepped)
     instead of rebuilding `[u32; W]` from `colv` at every last-row entry.
   - Check the asm for bounds checks in `rt.step` / `ct.step` / `bi[k]` (rust-skills
     `opt-bounds-check`). If they're there, try `get_unchecked` with a `// SAFETY:` note (W is a
     const, indices come from the trie builder).
4. **Find path:** a tall `find` (8×6: 11.6 s vs 2.5 s for 6×8) can't just be transposed, because
   the answer must stay naive's row-major first grid. But **proving "no solution"** (B13-type) can
   use the faster count kernel (count == 0) in either orientation. Also try count-style leaf
   handling in `search` for the exhaustive case.
5. **Bench noise:** huge-suite benches run once (±3%). Consider median of 3 for B10+ so failed
   experiments never look faster than the current code by luck (costs ~1 min per step).
6. **Bigger benchmarks** once B10 drops under a few seconds: count 5×6, find 7×8, prove no 6×9…
   (new IDs only, never change old ones).

## From the rust-skills checklist (relevant ones only)

| Rule | Status / idea |
|---|---|
| `opt-lto-release`, `opt-codegen-units`, `perf-release-profile` | ✅ release = fat LTO, 1 CGU, panic=abort |
| `opt-target-cpu` | ✅ target-cpu=native |
| `opt-pgo-profile` | tried, no gain (journal) |
| `opt-inline-always-rare` | `good`/`uni`/`step` are `inline(always)`. Re-measure each one. |
| `opt-inline-never-cold` | mark job setup / prefix replay / symmetry path `#[cold]` or `inline(never)`, to keep hot kernels small |
| `opt-bounds-check` | see idea 3: inspect asm first |
| `opt-simd-portable` | `std::simd` is nightly. Maybe `core::arch` AVX2 for the `good()` loop (8 letters at once via gathers)? Probably not worth it: sets are ~1–5 letters. |
| `opt-cache-friendly` | tries fit in L2. DFS layout failed (BFS keeps hot levels packed). |
| `mem-assert-type-size` | add `const _: () = assert!(size_of::<Node>() == 16 && size_of::<CNode>() == 8);` to catch layout regressions |
| `perf-profile-first` / `anti-premature-optimize` | keep doing: probe → measure → implement |
| `lint-*` | run `cargo clippy -- -W clippy::perf -W clippy::pedantic` once, fix what's cheap |
| `unsafe-safety-comment`, `unsafe-miri-ci` | if any `unsafe` gets in (idea 3), add SAFETY comments and run tests under miri on small sizes |

Not relevant here: async, serde, error crates (zero dependencies on purpose), rayon (std threads
already balance well).
