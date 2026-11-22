# Plan: things to figure out next

Working notes for the next optimization sessions. Every item that gets tried becomes a
benchmarked step (see CLAUDE.md). Failures too, labelled `(failed)`. Details of past steps:
[`results/journal.md`](results/journal.md). Main machine: **hephaestus** (Ryzen 9 3950X, 32 threads).

## Where we are (2026-11-22 day, after "resume aborted jobs")

| | B5 count 4×5 | B10 count 5×5 | B15 count 5×6 | B18 count 6×6 | B7 find 7×7 | B8 find 6×7 | B12 find 6×8 | B13 prove no 5×10 | B16 prove no 6×9 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| start of the night 11-21 | 1.20 s | 3.91 s | 13.47 s | 20.9 s | 134 ms | 117 ms | 1.01 s | 0.99 s | 9.93 s |
| morning 11-22 | 0.718 s | 3.96 s | 10.1 s | 18.0 s | 103 ms | 82 ms | 803 ms | 0.538 s | 6.04 s |
| now | **0.684 s** | 3.93 s | **9.86 s** | 17.8 s | **43 ms** | **33 ms** | **641 ms** | **0.445 s** | **4.64 s** |

Where the time goes now:
- **counts:** throughput-bound (31× CPU use on 32 threads), the second-to-last row's first cells
  (heatmap). Node counts mislead there: last-row nodes are counted in bulk and nearly free.
- **finds with a solution:** throughput of the job tests before the answer's job (timeline chart:
  6×8's first 192 jobs are ~550 ms of real work). The ~5 ms between "a thread is idle" and the long
  jobs aborting is the budget check: checking more often cost more than it saved.
- **prove none:** mirror-image job tests, exhaustive.

## Learned today (details in the journal)

- **Scheduling was a real lever for satisfiable finds:** time budgets per job, splitting only while a
  thread is idle, cutting later jobs as soon as a *test* says yes, splitting mirror tests in the mirror's
  order, and resuming aborted jobs where they stopped. Together: 7×7 2.2×, 6×7 2.5×, 6×8 1.24×.
- **Re-sweep thresholds after big changes:** the next-row set limit (8 → 10 letters) dated from
  before the mirror; proofs got 1.2–1.3× faster.
- **Register pressure is real:** keeping one extra value alive across the recursive call cost the
  kernel 3.5–4%, though the code using it never ran. Recover what you need on the cold path instead.
- **Measured and dead:** PGO (20–27% slower), memo on column tuples (all distinct), other letter
  orders (the mirror is within 1–2% of the best of all 120 for 4×5), cheap last-row end check (prunes
  65% of nodes on wide rows, saves no time), mirror job tests on small grids (mixed).

## Ideas, roughly by expected value

1. **Faster job tests = faster everything now.** Finds are throughput-bound on the count kernel's
   `ANY` mode. Any per-node gain in `count` shows up in every benchmark.
2. **A cheaper next-row set for wide rows** (still open): the DAWG multiset prunes 2× on 6×9 but costs
   2.7× per node; needs a layout where a child step costs about one trie step.
3. **Abort latency** (see the timeline): the cancel check runs when `nodes & 1023 == 0`, which the bulk
   node adds of the last rows skip over. A cheaper trigger (e.g. a per-thread flag set by the idle
   thread, checked only in shallow frames) might close the 5 ms gap.
4. **12-byte row node, `next2` dropped only where it doesn't prune** (it helped 4×6/5×6, not
   4×5/5×5). A per-width const choice, like `TRACK`.
5. **The second-to-last row on 5-row grids** (`count_pen` lost 3% there; maybe register pressure too,
   see "learned today").
6. **Bigger benchmarks:** B5–B8 are now under 0.7 s. Candidates: count 5×7 (9.9 s), count 7×7
   (30 s), prove no 7×8 (36 s), count 8×8 (18 s).
7. **The last two rows' dead ends** (was #0): strong pruning is easy, but every check measured so far
   costs more than the nearly free nodes it removes. Low expected value now.

## Done 2026-11-22 (day)

Split long find jobs ✅ · split mirror tests + early cut ✅ · next-row set up to 10 letters ✅ ·
last-row end check ❌ · first two rows swapped ✅ · count jobs 512 ✅ · resume aborted jobs ✅ · find budget 2 ms ✅.
Measured, no bar: PGO, column-tuple memo, letter orders, mirror on small grids, constants re-swept,
more frequent abort checks. New chart: find timeline (`scripts/timeline.py`).

## Done the night of 2026-11-21

Exact next-row set ✅ · find kernel set ❌ · column look-ahead on the set ❌ · reuse look-ahead
children ❌ · 12-byte packed nodes ❌ · count-kernel job test in find ✅ · wide-row set ❌ ·
second-to-last-row kernel + uninit buffers ✅ · set cap 32 ✅ · short find budget ✅ · DAWG set ❌ ·
mirrored rows for counts ✅ · anti-diagonal symmetry ✅ · skip-gram bound ❌ · mirror-image job
test in find ✅ · pair look-ahead ❌ · whole-word last-row check ❌ (strong pruning, too costly) · bigram path check ❌.
Also: `wordrect squares` (symmetric word squares) and a second dictionary atlas. Plus: B17–B18 (replayed for every step), whole history replayed on hephaestus, B15–B16, the atlas
(every size to 10×22 settled; largest = 8×8), gallery, anatomy, heatmap and scaling charts.

## From the rust-skills checklist (still open)

| Rule | Status / idea |
|---|---|
| `opt-inline-never-cold` | mark job setup / prefix replay / symmetry path `#[cold]`; re-measure `inline(always)` |
| `lint-*` | `cargo clippy -- -W clippy::perf -W clippy::pedantic` once, fix what's cheap |
| `unsafe-miri-ci` | run `cargo +nightly miri test` on a tiny dict once (there is `unsafe` in the kernel) |
| `opt-pgo-profile` | retried 2026-11-22: 20–27% slower. Dropped. |
