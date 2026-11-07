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
| `opt-bounds-check` | ✅ done 2026-11-07 ("unchecked trie steps", ~5%). Left to check: `colv[c]`, `masks[c]`, `st.grid[cell]` (runtime `c`; asm shows some checks are already folded) |
| `opt-simd-portable` | `std::simd` is nightly. Maybe `core::arch` AVX2 for the `good()` loop (8 letters at once via gathers)? Probably not worth it: sets are ~1–5 letters. |
| `opt-cache-friendly` | tries fit in L2. DFS layout failed (BFS keeps hot levels packed). |
| `mem-assert-type-size` | ✅ done (const assert in `par.rs`) |
| `perf-profile-first` / `anti-premature-optimize` | keep doing: probe → measure → implement |
| `lint-*` | run `cargo clippy -- -W clippy::perf -W clippy::pedantic` once, fix what's cheap |
| `unsafe-safety-comment`, `unsafe-miri-ci` | SAFETY comments ✅ (`rstep`/`cstep`). TODO: run `cargo +nightly miri test` on a tiny dict (3×3) once, since `unsafe` is now in the kernel |

Not relevant here: async, serde, error crates (zero dependencies on purpose), rayon (std threads
already balance well).

## Next session (written 2026-11-07, after "unchecked trie steps")

Current: B10 count 5×5 9.81 s, B5 4×5 3.07 s, B14 6×4 5.55 s (fast profile).
Ordered by how much I expect, plus the rust-skills rule behind each one:

1. **Exact last-row check while the set is small** (idea 1 above). Still the biggest lever:
   94–97% of second-to-last-row nodes are dead. *Skills: `perf-profile-first`: probe how often
   the set has ≤ 8 nodes before coding it.*
2. **Reuse `good()`'s children** (idea 3): `good` already does `rstep(row, l)` for every letter,
   then the loop steps again. Keep the children in a small `[Node; 26]` stack buffer, or do the
   loop on them directly. *Skills: `mem-arrayvec` idea without the crate (fixed array + len),
   `opt-cache-friendly`.*
3. **Remaining bounds checks** on `colv[c]` / `masks[c]`: pass `c` as a const-generic-friendly
   index, or assert `c < W` once at the top of `count` so LLVM drops later checks.
   *Skills: `opt-bounds-check` (verify in asm: `objdump -d -C target/fast/wordrect`, look for
   `call *…(%rip)` panic stubs in `Kernel<5>::count`).*
4. **`#[cold]` / `#[inline(never)]`** on `jobs`, `load`, `to_grid`, the sym tie path. Then
   re-measure each `inline(always)` (`good`, `uni`, `rstep`). *Skills: `opt-inline-never-cold`,
   `opt-cold-unlikely`, `opt-inline-always-rare`.*
5. **Clippy pass:** `cargo clippy --all-targets -- -W clippy::perf -W clippy::pedantic`, fix only
   what's cheap and doesn't touch `naive.rs`. Also `-W clippy::undocumented_unsafe_blocks`.
   *Skills: `lint-warn-perf`, `lint-pedantic-selective`, `lint-unsafe-doc`.*
6. **Miri** on a tiny instance now that there is `unsafe`. *Skills: `unsafe-miri-ci`.*
7. **Median of 3 for the huge benches** (idea 5): wins are now ~5%, close to the noise.
   *Skills: `perf-black-box-bench` mindset: trustworthy measurements first.*
8. **Release-profile official run** (`./wr release --label ...`) once a few steps have piled
   up, and a PGO retry, since the kernel changed a lot. *Skills: `opt-pgo-profile`.*
