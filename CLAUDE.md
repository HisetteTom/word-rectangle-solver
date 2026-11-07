# CLAUDE.md

Ultra-optimized word rectangle solver in Rust. Plan, roadmap and rules: `README.md`.
Launcher: `./wr` (`./wr help`). Zero dependencies — keep it that way.

## Hard rules

- **Never optimize `src/naive.rs`.** It is the correctness reference and the speed baseline.
- Every new solver must agree **exactly** with naive: add it to `tests/cross_check.rs`.
- Every solution printed is checked by `src/validate.rs` (shares no code with solvers).

## Always record benchmarks — after EVERY optimization (kept or reverted)

The benchmark history is the point of this project; it is published on GitHub as charts.

1. `./wr test` must pass.
2. **Commit the code change first**, so the log row points at real code (bench warns on `-dirty`).
3. `./wr bench --label "<short step name>"` — a new, unique label per step
   (e.g. `"trie + masks"`, `"BFS node order"`). No commas. This appends to
   `results/bench-log.csv` and regenerates `results/charts/*.svg` + `results/BENCHMARKS.md`.
   Never edit those generated files or old CSV rows by hand.
4. Add an entry to `results/journal.md` (idea, before/after table with × faster, verdict),
   including failed experiments. New record grids go in `results/records.md`.
5. Commit the results (`bench-log.csv`, `charts/`, `BENCHMARKS.md`, `journal.md`) and **push**,
   so the charts on GitHub show the new step.

Reverted experiments: still run step 3 on the experimental commit (the bar documents it), then revert.

Suites: `./wr bench quick` (B1–B3, seconds) for fast iteration while developing; the recorded step
must be a full `./wr bench` (~1 min now) so every chart gets the new bar.
Use `./wr release --label ...` for official numbers; `fast` profile is fine day to day.
When a benchmark drops below ~1 ms, add a bigger benchmark (`B4`, …) to `BENCHES` in
`scripts/bench.sh` instead of changing existing ones — old IDs must keep meaning the same task.
