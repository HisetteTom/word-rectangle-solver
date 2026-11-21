# Records

Dictionary: `enable.txt` — SHA-256 `3f161302…1a89` (see `dicts/HASHES.txt`).

## Solutions found

| Date | Dictionary | Ruleset | Size | Solution | Search time | Solver | Commit |
|---|---|---|---|---|---|---|---|
| 2026-11-07 | enable | rectangle, repeats ok | 6×8 | `acidemia/ninepins/unslings/reticule/imamates/careless` | 5.10 s | by-value node kernel (par) | 6da98c6 |
| 2026-11-07 | enable | rectangle, repeats ok | 7×7 | `abasers/bedemen/adenoma/senator/emotive/remover/snarers` | 3.96 s | compact trie | d3df964 |
| 2026-11-07 | enable | rectangle, repeats ok | 6×7 | `abasers/manatee/trishaw/relieve/aginner/censers` | 3.27 s | compact trie | d3df964 |
| 2026-11-07 | enable | rectangle, repeats ok | 5×9 | `aggravate/marinaras/pluckiest/lamellate/expresser` | 17.8 s | compact trie | d3df964 |
| 2026-11-07 | enable | rectangle, repeats ok | 6×6 | `aahing/agonal/hoodie/indole/nailed/gleeds` | 5.4 s | naive | initial |
| 2026-11-07 | enable | rectangle, repeats ok | 5×6 | `aahing/acedia/ramees/greave/hisses` | 2.4 s | naive | initial |
| 2026-11-07 | enable | rectangle, repeats ok | 6×6 | `aahing/agonal/hoodie/indole/nailed/gleeds` | 1.9 ms | trie | 60c0927 |

## Exhausted sizes (proven: no solution)

| Date | Dictionary | Ruleset | Size | Nodes visited | Time | Solver | Commit |
|---|---|---|---|---|---|---|---|
| 2026-11-07 | enable | rectangle, repeats ok | 5×10 | 3,802,792,308 | 4.94 s | by-value node kernel (par, 22 threads) | 6da98c6 |

## Solution counts (exhaustive)

| Date | Dictionary | Ruleset | Size | Solutions | Time | Solver | Commit |
|---|---|---|---|---|---|---|---|
| 2026-11-07 | enable | rectangle, repeats ok | 3×3 | 1,909,732 | 6.4 ms | par | 6da98c6 |
| 2026-11-07 | enable | rectangle, repeats ok | 4×4 | 136,245,247 | 1.12 s | par | 6da98c6 |
| 2026-11-07 | enable | rectangle, repeats ok | 4×5 | 114,846,713 | 4.84 s | par | 6da98c6 |
| 2026-11-07 | enable | rectangle, repeats ok | 3×7 | 4,453,472 | 0.48 s | par | 6da98c6 |
| 2026-11-07 | enable | rectangle, repeats ok | 3×8 | 989,479 | 0.48 s | par | 6da98c6 |
| 2026-11-07 | enable | rectangle, repeats ok | 4×6 | 37,812,638 | 9.50 s | par | 6da98c6 |
| 2026-11-07 | enable | rectangle, repeats ok | 5×5 | 80,364,530 | 30.6 s | par | 6da98c6 |

## The atlas (2026-11-21, hephaestus, `scripts/atlas.py`)

Every size from 2×2 to 10×22 is now settled: an exact count for every size that has solutions, a
proof for every size that has none. Chart: [`charts/atlas.svg`](charts/atlas.svg), data:
[`atlas.csv`](atlas.csv), the rarest grids: [`charts/gallery.svg`](charts/gallery.svg).

**Largest rectangle in ENABLE: 8×8.** There are exactly **5** of them, and all five are symmetric word
squares (each row equals the matching column). So no non-symmetric 8×8 exists, and nothing at 9×9,
8×9, 7×8 (!), 6×9, 5×10, 4×12, 3×14 or 2×14 and beyond.

| Size | Solutions | Time (release, 32 threads) | Note |
|---|---:|---:|---|
| 8×8 | 5 | 27.8 s | `crabwise/ratlines/atlantes/blastema/winterly/intertie/seemlier/essayers` and 4 more, all symmetric |
| 7×7 | 11,669 | 42.4 s | |
| 7×8 | 0 | 60.1 s | proven impossible, even though 8×8 exists |
| 6×6 | 1,584,465 | 20.2 s | |
| 6×7 | 3,584 | 50.3 s | |
| 6×8 | 9 | 39.1 s | |
| 5×7 | 440,988 | 15.1 s | |
| 5×8 | 10,700 | 11.7 s | |
| 5×9 | 42 | 3.40 s | |
| 4×11 | 2 | 0.18 s | `gastrotrich/agoraphobia/delineating/seaminesses`, `scleroderma/reanimation/intolerable/stillnesses` |
| 3×13 | 1 | 0.10 s | `acetazolamide/aeromedicines/serriednesses`: the only one |
| 2×13 | 2 | 0.09 s | `antihistamine/guanethidines`, `antisatellite/leishmaniasis` |
| 9×9, 10×10 | 0 | 3.5 s, 0.5 s | proven impossible |
