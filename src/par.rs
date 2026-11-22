//! Level 6: parallel version of the row-major trie search (`search.rs`), std threads only.
//! The hot loops (`Kernel`) carry trie nodes by value and track (row, col) directly.
//! Square counts use transpose symmetry (`Kernel::count_sym`): about half the nodes.
//! Row look-ahead: a letter is skipped when the row node it leads to (`next`, `next2` masks)
//! can't fit the columns two and three cells further right (forward checking).
//! Next-row set: while filling row r, the count kernel tracks the exact set of row-trie nodes row
//! r + 1 can still be in (up to `S_CAP`), which prunes harder than bigrams and, in the second-to-last
//! row, counts the last rows directly (no last-row walk).
//!
//! 0. `find` first tries a short sequential search (`FIND_SEQ_BUDGET` nodes): small cases stay cheap.
//! 1. Split: expand the first cells breadth-first (lexicographic order) until there are
//!    enough jobs (`*_JOBS_PER_THREAD` × threads). A job = the letters of that prefix.
//! 2. Workers pull jobs from an atomic counter (dynamic load balancing: subtrees are
//!    wildly uneven, and P-cores / E-cores run at different speeds).
//! 3. `find` is deterministic: jobs are ranked in lexicographic order, and workers skip / abort
//!    any job ranked after one known to hold a solution, so the answer is the lexicographically
//!    first grid, exactly what the sequential solver returns. Long jobs are split while threads
//!    are idle (see `find`).

use crate::trie::{CNode, CTrie, Node, Trie};

// Layout guard (rust-skills `mem-assert-type-size`): the kernels are tuned for these sizes.
const _: () = assert!(size_of::<Node>() == 16 && size_of::<CNode>() == 8);
use crate::{Grid, Solver, Stats};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed, Ordering::SeqCst};
use std::time::{Duration, Instant};

/// Calls `$body` with the const generic `$W` = `$w` (1..=32): the kernels are monomorphized per
/// row width, so column loops, `c + k < W` tests and array bounds are compile-time constants.
macro_rules! by_width {
    ($w:expr, |$W:ident| $body:expr) => {
        by_width!(@ $w, $W, $body, 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32)
    };
    (@ $w:expr, $W:ident, $body:expr, $($n:literal)*) => {
        match $w {
            $($n => { const $W: usize = $n; $body })*
            w => panic!("par supports 1..=32 columns, got {w}"),
        }
    };
}

/// Jobs per thread. Exhaustive counting balances fine with few jobs (512: 1–3% faster than 256 on
/// 4×5, 4×6, 4×7, 5×5 since the kernels got faster; 1024 costs 3×7 25% in job expansion); `find`
/// needs many small jobs, because the job that holds the first solution must not be one huge subtree.
const COUNT_JOBS_PER_THREAD: usize = 512;
const FIND_JOBS_PER_THREAD: usize = 4096;
/// `find` first searches sequentially for this many nodes (~0.5 ms); small instances finish
/// there without paying for threads and job expansion.
const FIND_SEQ_BUDGET: u64 = 30_000;
/// Time budget of one parallel `find` job before it is split (see `find`), in µs. Measured: 1 ms
/// splits too often, 2–10 ms are about equal.
const FIND_JOB_BUDGET_US: u64 = 5_000;
/// Largest exact next-row set the count kernel tracks (see `Kernel::count`). 32 since the buffer is
/// uninitialized (5×6: 12.44 s at 16, 11.99 s at 32; 5×5 unchanged).
const S_CAP: usize = 32;
/// Same, in the second-to-last row (`Kernel::count_pen`), where overflowing is costlier.
const S_CAP_PEN: usize = 128;
/// `pm` value meaning "`ps` is the set itself" (bit 31 is never a letter).
const SELF: u32 = 1 << 31;

pub struct ParSearch {
    rows: usize,
    cols: usize,
    row_trie: Trie,
    col_trie: CTrie,
    threads: usize,
    seq_budget: u64,
    /// Square with the same words for rows and columns: transposes of solutions are solutions.
    sym: bool,
    /// Per position k of a row word: which letters can follow at k + 1 (see `Bigram`).
    bigrams: Vec<Bigram>,
    /// The word lists, to build the re-oriented searches `count` uses (see `count`).
    row_words: Vec<String>,
    col_words: Vec<String>,
    /// This search counts the horizontal mirror image of the asked grid (row words reversed).
    mirrored: bool,
    /// Mirror image of a square with one word list: count with anti-diagonal symmetry.
    anti: bool,
    /// `find`: build the mirror for the job tests before starting (for tests; normally it is
    /// built on a detached thread and used once ready).
    sync_mirror: bool,
    /// `find`: split every job at once, as if all were over budget (for tests).
    eager_split: bool,
    stats: Stats,
}

/// A queued piece of `find` work, ranked by `lo` (see `find`; the queue serves the lowest first).
struct Task {
    lo: u64,
    hi: u64,
    budget: Duration,
    kind: TaskKind,
}

enum TaskKind {
    /// Test the job `prefix` (on the mirror if `mirror_ok` and it is built) and search it if it has
    /// solutions.
    Job { prefix: Vec<u8>, mirror_ok: bool },
    /// Part of the mirror test of job `prefix`: does the mirror prefix `m` extend to a grid? `group`
    /// is set once a part of this job found one (the job is then searched, once).
    Part { group: std::sync::Arc<AtomicBool>, prefix: Vec<u8>, m: Vec<u8> },
}

impl PartialEq for Task {
    fn eq(&self, o: &Self) -> bool {
        self.lo == o.lo
    }
}
impl Eq for Task {}
impl PartialOrd for Task {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Task {
    /// Reversed: `BinaryHeap` pops the greatest, the queue must pop the lowest rank.
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        o.lo.cmp(&self.lo)
    }
}

/// `find`'s job cut: (rank bound: work ranked at or above it is useless, this job's rank, deadline).
type Cut<'a> = (&'a AtomicU64, u64, Option<(Instant, &'a AtomicBool)>);

/// Per-thread search state.
struct Worker<'a> {
    s: &'a ParSearch,
    row_state: Vec<u32>,
    col_state: Vec<u32>,
    grid: Vec<u8>,
    nodes: u64,
    /// `search` gives up once `nodes` exceeds this (checked every 256 nodes, with `stop`).
    limit: u64,
    /// The last `count_job` was cut short (its result is a lower bound).
    cancelled: bool,
}

impl<'a> Worker<'a> {
    fn new(s: &'a ParSearch) -> Self {
        Worker { s, row_state: vec![0; s.rows], col_state: vec![0; s.cols], grid: vec![0; s.rows * s.cols], nodes: 0, limit: u64::MAX, cancelled: false }
    }

    /// Reset to the empty grid and replay a job prefix.
    fn load(&mut self, prefix: &[u8]) {
        self.row_state.fill(0);
        self.col_state.fill(0);
        for (cell, &l) in prefix.iter().enumerate() {
            let (r, c) = (cell / self.s.cols, cell % self.s.cols);
            self.row_state[r] = self.s.row_trie.child(self.row_state[r], l as usize);
            self.col_state[c] = self.s.col_trie.child(self.col_state[c], l as usize);
            self.grid[cell] = b'a' + l;
        }
    }

    #[inline(always)]
    fn allowed(&self, cell: usize) -> (usize, usize, u32, u32, u32) {
        let (r, c) = (cell / self.s.cols, cell % self.s.cols);
        let (rn, cn) = (self.row_state[r], self.col_state[c]);
        (r, c, rn, cn, self.s.row_trie.mask(rn) & self.s.col_trie.mask(cn))
    }

    /// Count all completions of the loaded job prefix (`cell` = first empty cell).
    /// `plain` = Some(cut) for `find`: no square symmetry (prefixes leave row 0), stop at the first
    /// solution (the result is then only "0" or "some"), and give up (0, `cancelled` set) when
    /// the cut says so (see `Cut`).
    fn count_job(&mut self, cell: usize, plain: Option<Cut>) -> u64 {
        by_width!(self.s.cols, |W| self.count_job_w::<W>(cell, plain))
    }

    fn count_job_w<const W: usize>(&mut self, cell: usize, plain: Option<Cut>) -> u64 {
        let s = self.s;
        let (r, c) = (cell / W, cell % W);
        let mut colv: [CNode; W] = std::array::from_fn(|i| s.col_trie.node(self.col_state[i]));
        let mut k = Kernel::<W>::new(s);
        k.cut = plain;
        // separate instances with and without `count_pen`: even an unused call site slowed the
        // generic kernel of 5-row grids by 2–3%
        let pen = Kernel::<W>::PEN && s.rows <= 4;
        let mut nodes = 0;
        let row = s.row_trie.node(self.row_state[r]);
        let total = if (s.sym || s.anti) && plain.is_none() {
            // job prefixes stay inside row 0 when `sym`/`anti` (see `count`), so they are all "tied"
            let mut row0 = [0u8; W];
            for (d, &g) in row0.iter_mut().zip(&self.grid[..cell]) {
                *d = g - b'a';
            }
            match (s.anti, pen) {
                (true, true) => k.count_anti::<true>(r, c, row, &mut colv, &mut nodes, &mut row0),
                (true, false) => k.count_anti::<false>(r, c, row, &mut colv, &mut nodes, &mut row0),
                (false, true) => k.count_sym::<true>(r, c, row, &mut colv, &mut nodes, &mut row0),
                (false, false) => k.count_sym::<false>(r, c, row, &mut colv, &mut nodes, &mut row0),
            }
        } else {
            let ps: &[Node] = if c == 0 { std::slice::from_ref(&k.root) } else { &[] };
            match (plain.is_some(), pen) {
                (true, true) => k.count::<true, true>(r, c, row, &mut colv, &mut nodes, !0, ps, SELF),
                (true, false) => k.count::<true, false>(r, c, row, &mut colv, &mut nodes, !0, ps, SELF),
                (false, true) => k.count::<false, true>(r, c, row, &mut colv, &mut nodes, !0, ps, SELF),
                (false, false) => k.count::<false, false>(r, c, row, &mut colv, &mut nodes, !0, ps, SELF),
            }
        };
        self.nodes += nodes;
        self.cancelled = k.cancelled.get();
        total
    }

    /// Calls `on` on every solution of the loaded job prefix (`cell` = first empty cell);
    /// returns false if stopped (`on` returned false, `stop()` became true, or the node limit hit).
    fn search<F: FnMut(&[u8]) -> bool, S: Fn() -> bool>(&mut self, cell: usize, on: &mut F, stop: &S) -> bool {
        by_width!(self.s.cols, |W| self.search_w::<W, F, S>(cell, on, stop))
    }

    fn search_w<const W: usize, F: FnMut(&[u8]) -> bool, S: Fn() -> bool>(&mut self, cell: usize, on: &mut F, stop: &S) -> bool {
        let s = self.s;
        if cell == s.rows * W {
            return on(&self.grid);
        }
        let (r, c) = (cell / W, cell % W);
        let k = Kernel::<W>::new(s);
        let mut st = SearchState::<W> {
            colv: std::array::from_fn(|i| s.col_trie.node(self.col_state[i])),
            grid: std::mem::take(&mut self.grid),
            nodes: self.nodes,
            limit: self.limit,
        };
        let done = k.search(r, c, s.row_trie.node(self.row_state[r]), &mut st, on, stop, !0);
        self.grid = st.grid;
        self.nodes = st.nodes;
        done
    }
}

/// Hot counting recursion. Carries trie nodes *by value*: `allowed` needs no memory
/// load, only the two child steps do. (r, c) are tracked directly (no `/` or `%`), and
/// the row state is a single node (finished rows are never needed again).
struct Kernel<'t, const W: usize> {
    /// Row trie nodes, as a plain slice (one load less per step than going through `&Trie`).
    rt: &'t [Node],
    /// `ParSearch::bigrams`: exactly one table per row position.
    bi: &'t [Bigram; W],
    /// Column trie nodes.
    ct: &'t [CNode],
    /// Row trie root, cached (no load + bounds check at every row start).
    root: Node,
    last_r: usize,
    /// `count::<ANY>` only (see `Cut`). The job is cancelled once an earlier job is known to hold a
    /// solution, or the deadline passed while a thread is idle; `cancelled` then makes every frame
    /// return at once.
    cut: Option<Cut<'t>>,
    cancelled: std::cell::Cell<bool>,

}

impl<'t, const W: usize> Kernel<'t, W> {
    fn new(s: &'t ParSearch) -> Self {
        let bi = s.bigrams.as_slice().try_into().expect("one bigram table per column");
        Kernel { rt: s.row_trie.nodes(), bi, ct: s.col_trie.nodes(), root: s.row_trie.node(0), last_r: s.rows - 1, cut: None, cancelled: std::cell::Cell::new(false) }
    }

    /// Row child of `n` for letter `l`, without a bounds check (rust-skills `opt-bounds-check`).
    ///
    /// # Safety
    /// `n` is a node of the row trie and bit `l` of `n.mask` is set. Then the child exists, and
    /// `Trie::new` stored it at `first + popcount(lower letters)` < `rt.len()`.
    #[inline(always)]
    unsafe fn rstep(&self, n: Node, l: u32) -> Node {
        let i = Trie::child_idx(n, l) as usize;
        debug_assert!(n.mask >> l & 1 == 1 && i < self.rt.len());
        // SAFETY: see the function contract above.
        unsafe { *self.rt.get_unchecked(i) }
    }

    /// Column child of `n` for letter `l`, without a bounds check.
    ///
    /// # Safety
    /// Same contract as `rstep`, for the column trie.
    #[inline(always)]
    unsafe fn cstep(&self, n: CNode, l: u32) -> CNode {
        let i = (n.first + (n.mask & ((1u32 << l) - 1)).count_ones()) as usize;
        debug_assert!(n.mask >> l & 1 == 1 && i < self.ct.len());
        // SAFETY: see the function contract above.
        unsafe { *self.ct.get_unchecked(i) }
    }
}

impl<const W: usize> Kernel<'_, W> {
    /// Track the exact next-row set only on rows of up to 10 letters. Set at 8 before the mirror
    /// image (rows filled from their last letter), when it overflowed `S_CAP` almost at once on
    /// longer rows (6×9 count: 10.5 s with it, 9.4 s without). On the mirror it pays up to 10
    /// (6×9 count 6.9 -> 5.4 s, prove no 6×9 6.1 -> 4.6 s, 5×10 0.55 -> 0.45 s); above that it is even.
    const TRACK: bool = W <= 10;
    /// Use `count_pen` for the second-to-last row only where it measured faster: grids of at most
    /// 4 rows and 6 columns (`P` = true instances of the kernels). On 4×7, 5×5, 5×6, 5×7 and 6×6 it
    /// was 1.5–4% slower than the generic kernel, for no reason I could isolate without a profiler.
    const PEN: bool = W <= 6;

    /// Last row, cell `c`: completions of `row` with letter `c'` in `masks[c']` for every c' >= c.
    fn count_last(&self, c: usize, row: Node, masks: &[u32; W], nodes: &mut u64) -> u64 {
        *nodes += 1;
        crate::probe::hit(crate::probe::N - 1, c);
        let mut allowed = row.mask & masks[c];
        if c == (W - 1) {
            return allowed.count_ones() as u64;
        }
        let mut total = 0;
        if c + 1 == (W - 1) {
            // unrolled: the next cell is the last one, count it by popcount directly
            let last = masks[c + 1];
            *nodes += allowed.count_ones() as u64;
            while allowed != 0 {
                let l = allowed.trailing_zeros();
                allowed &= allowed - 1;
                // SAFETY: `l` comes from `allowed`/`good`, subsets of this node's mask.
                total += (unsafe { self.rstep(row, l) }.mask & last).count_ones() as u64;
            }
            return total;
        }
        let m = |k: usize| if k < W { masks[k] } else { !0 };
        let mut good = self.good(row, allowed, c, m(c + 1), m(c + 2), m(c + 3));
        while good != 0 {
            let l = good.trailing_zeros();
            good &= good - 1;
            // SAFETY: `l` comes from `allowed`/`good`, subsets of this node's mask.
            total += self.count_last(c + 1, unsafe { self.rstep(row, l) }, masks, nodes);
        }
        total
    }
}

impl<const W: usize> Kernel<'_, W> {
    /// Transpose symmetry for squares (rows and columns share one word list).
    /// Transposing swaps (first row, first column), so
    ///   count = 2 * #{row0 < col0} + #{row0 == col0}.
    /// This runs only on the "tied" path, where column 0 equals row 0 so far. At (r, 0), letters
    /// below `row0[r]` are cut, the equal letter stays tied, and greater letters leave for the
    /// plain kernel with weight 2.
    fn count_sym<const P: bool>(&self, r: usize, c: usize, row: Node, colv: &mut [CNode; W], nodes: &mut u64, row0: &mut [u8; W]) -> u64 {
        if r == self.last_r {
            let mut masks = [0u32; W];
            for (m, n) in masks.iter_mut().zip(colv.iter()) {
                *m = n.mask;
            }
            if c == 0 && r >= 1 {
                let w = row0[r] as u32;
                let m0 = masks[0];
                masks[0] = m0 & (1 << w);
                let eq = self.count_last(0, row, &masks, nodes);
                masks[0] = m0 & !((2u32 << w) - 1);
                let gt = self.count_last(0, row, &masks, nodes);
                return eq + 2 * gt;
            }
            return self.count_last(c, row, &masks, nodes);
        }
        *nodes += 1;
        crate::probe::hit(r, c);
        let col = colv[c];
        let mut allowed = row.mask & col.mask;
        let tie_cell = c == 0 && r >= 1;
        let w = row0[r] as u32;
        if tie_cell {
            allowed &= !((1u32 << w) - 1); // letters >= row0[r]
        }
        let mut total = 0;
        while allowed != 0 {
            let l = allowed.trailing_zeros();
            allowed &= allowed - 1;
            if r == 0 {
                row0[c] = l as u8;
            }
            // SAFETY: `l` comes from `allowed`/`good`, subsets of this node's mask.
            colv[c] = unsafe { self.cstep(col, l) };
            // SAFETY: `l` comes from `allowed`/`good`, subsets of this node's mask.
            let (nr, nc, nrow) = if c == (W - 1) { (r + 1, 0, self.root) } else { (r, c + 1, unsafe { self.rstep(row, l) }) };
            total += if tie_cell && l > w {
                2 * self.count::<false, P>(nr, nc, nrow, colv, nodes, !0, if nc == 0 { std::slice::from_ref(&self.root) } else { &[] }, SELF)
            } else {
                self.count_sym::<P>(nr, nc, nrow, colv, nodes, row0)
            };
        }
        colv[c] = col;
        total
    }
}

impl<const W: usize> Kernel<'_, W> {
    /// Anti-diagonal symmetry, for the **mirror image** of a square (rows = reversed words, columns =
    /// words; see `ParSearch::count`). Reflecting it across the anti-diagonal maps solutions to
    /// solutions: row i becomes column W-1-i read bottom-up (a reversed word), column j becomes row
    /// W-1-j read right-to-left (a word). With A = row 0 read right-to-left and B = the last column
    /// read top-down (the reflection swaps them),
    ///   count = 2 * #{A < B} + #{A == B}.
    /// Like `count_sym`, this runs only on the tied path; the tie cells are (r, W-1), where
    /// B[r] meets A[r] = row0[W-1-r].
    fn count_anti<const P: bool>(&self, r: usize, c: usize, row: Node, colv: &mut [CNode; W], nodes: &mut u64, row0: &mut [u8; W]) -> u64 {
        if r == self.last_r {
            let mut masks = [0u32; W];
            for (m, n) in masks.iter_mut().zip(colv.iter()) {
                *m = n.mask;
            }
            // the last cell of the last row is the last tie cell
            let w = row0[W - 1 - r] as u32;
            let m = masks[W - 1];
            masks[W - 1] = m & (1 << w);
            let eq = self.count_last(c, row, &masks, nodes);
            masks[W - 1] = m & !((2u32 << w) - 1);
            let gt = self.count_last(c, row, &masks, nodes);
            return eq + 2 * gt;
        }
        *nodes += 1;
        crate::probe::hit(r, c);
        let col = colv[c];
        let mut allowed = row.mask & col.mask;
        let tie_cell = c == W - 1 && r >= 1;
        let w = row0[W - 1 - r] as u32;
        if tie_cell {
            allowed &= !((1u32 << w) - 1); // letters >= A[r]
        }
        let mut total = 0;
        while allowed != 0 {
            let l = allowed.trailing_zeros();
            allowed &= allowed - 1;
            if r == 0 {
                row0[c] = l as u8;
            }
            // SAFETY: `l` comes from `allowed`, a subset of this node's mask.
            colv[c] = unsafe { self.cstep(col, l) };
            // SAFETY: as above.
            let (nr, nc, nrow) = if c == (W - 1) { (r + 1, 0, self.root) } else { (r, c + 1, unsafe { self.rstep(row, l) }) };
            total += if tie_cell && l > w {
                2 * self.count::<false, P>(nr, nc, nrow, colv, nodes, !0, if nc == 0 { std::slice::from_ref(&self.root) } else { &[] }, SELF)
            } else {
                self.count_anti::<P>(nr, nc, nrow, colv, nodes, row0)
            };
        }
        colv[c] = col;
        total
    }
}

/// Mutable state of `Kernel::search`.
struct SearchState<const W: usize> {
    colv: [CNode; W],
    grid: Vec<u8>,
    nodes: u64,
    limit: u64,
}

impl<const W: usize> Kernel<'_, W> {
    /// Same order as `count`, writes letters to the grid; false = stop.
    /// `f`: forward mask of the next row, as in `count`.
    #[allow(clippy::too_many_arguments)]
    fn search<F: FnMut(&[u8]) -> bool, S: Fn() -> bool>(&self, r: usize, c: usize, row: Node, st: &mut SearchState<W>, on: &mut F, stop: &S, f: u32) -> bool {
        st.nodes += 1;
        crate::probe::hit(r, c);
        if st.nodes & 255 == 0 && (st.nodes > st.limit || stop()) {
            return false;
        }
        let col = st.colv[c];
        let mut allowed = row.mask & col.mask;
        if c < W - 1 {
            // look-ahead: drop letters whose row can't fit the columns further right
            let m = |k: usize| if k < W { st.colv[k].mask } else { !0 };
            allowed = self.good(row, allowed, c, m(c + 1), m(c + 2), m(c + 3));
        }
        let last_row = r == self.last_r;
        // letters row r + 1 could have at position c (no next row in the last row)
        let base = if last_row { !0 } else if c == 0 { self.root.mask } else { self.uni(c - 1, f) };
        let cell = r * W + c;
        let mut keep_going = true;
        while allowed != 0 {
            let l = allowed.trailing_zeros();
            allowed &= allowed - 1;
            st.grid[cell] = b'a' + l as u8;
            let ok = if last_row && c == W - 1 {
                on(&st.grid)
            } else {
                // SAFETY: `l` comes from `allowed`/`good`, subsets of this node's mask.
                let cn = unsafe { self.cstep(col, l) };
                let fc = if last_row { !0 } else { base & cn.mask };
                if fc == 0 {
                    continue; // next row can't fit the columns
                }
                st.colv[c] = cn;
                if c == W - 1 {
                    self.search(r + 1, 0, self.root, st, on, stop, !0)
                } else {
                    // SAFETY: `l` comes from `allowed`/`good`, subsets of this node's mask.
                    self.search(r, c + 1, unsafe { self.rstep(row, l) }, st, on, stop, fc)
                }
            };
            if !ok {
                keep_going = false;
                break;
            }
        }
        st.colv[c] = col;
        keep_going
    }

    /// `f` = forward mask of the *next* row (r + 1) at position c - 1: the letters a row word can
    /// have there, given what the columns already force on positions 0..c-1 of row r + 1
    /// (any superset is valid, e.g. `!0`). Unused at c == 0.
    /// `ANY`: return as soon as the total is non-zero (an existence test, for `find`).
    /// `P`: hand the second-to-last row to `count_pen` (see `Kernel::PEN`).
    /// `ps`, `pm` = the exact set S of row-trie nodes (depth c) that row r + 1 can still be in, given
    /// the column letters already forced on its positions 0..c-1: S = children of the nodes `ps` by
    /// letters in `pm`, or `ps` itself when `pm == SELF`. Built lazily, only if this cell has a live
    /// letter. `ps` empty = unknown (S had more than `S_CAP` nodes): the bigram bound `f` is used.
    #[allow(clippy::too_many_arguments)]
    fn count<const ANY: bool, const P: bool>(&self, r: usize, c: usize, row: Node, colv: &mut [CNode; W], nodes: &mut u64, f: u32, ps: &[Node], pm: u32) -> u64 {
        if r == self.last_r {
            // Last row: columns only need their masks (their children are never used).
            let mut masks = [0u32; W];
            for (m, n) in masks.iter_mut().zip(colv.iter()) {
                *m = n.mask;
            }
            return self.count_last(c, row, &masks, nodes);
        }
        if P && r + 1 == self.last_r && !ps.is_empty() {
            return self.count_pen::<ANY, P>(c, row, colv, nodes, f, ps, pm);
        }
        *nodes += 1;
        crate::probe::hit(r, c);
        if ANY {
            if *nodes & 1023 == 0
                && let Some((bound, j, deadline)) = self.cut
                && (bound.load(Relaxed) <= j || deadline.is_some_and(|(d, hurry)| hurry.load(Relaxed) && Instant::now() > d))
            {
                self.cancelled.set(true);
            }
            if self.cancelled.get() {
                return 0;
            }
        }
        let col = colv[c];
        let allowed = row.mask & col.mask;
        let mut good = if c == (W - 1) {
            allowed
        } else {
            let m = |k: usize| if k < W { colv[k].mask } else { !0 };
            self.good(row, allowed, c, m(c + 1), m(c + 2), m(c + 3))
        };
        if good == 0 {
            return 0;
        }
        // materialize S (or give up on it if it is too big)
        let mut buf = SetBuf::<S_CAP>::new();
        let (s, base): (&[Node], u32) = if !Self::TRACK || ps.is_empty() {
            (&[], if c == 0 { self.root.mask } else { self.uni(c - 1, f) })
        } else if pm == SELF {
            (ps, ps.iter().fold(0, |a, n| a | n.mask))
        } else {
            let mut n = 0;
            let mut base = 0;
            'fill: for sn in ps {
                let mut m = sn.mask & pm;
                while m != 0 {
                    let x = m.trailing_zeros();
                    m &= m - 1;
                    if n == S_CAP {
                        n = 0;
                        break 'fill;
                    }
                    // SAFETY: `x` is a bit of `sn.mask`, `sn` a row-trie node.
                    let ch = unsafe { self.rstep(*sn, x) };
                    base |= ch.mask;
                    buf.set(n, ch);
                    n += 1;
                }
            }
            if n == 0 { (&[], self.uni(c - 1, f)) } else { (buf.first(n), base) }
        };
        let mut total = 0;
        if c == (W - 1) {
            // the letter completes this row: next cell is (r + 1, 0) with a fresh row
            let pen = r + 1 == self.last_r && !s.is_empty();
            while good != 0 {
                let l = good.trailing_zeros();
                good &= good - 1;
                // SAFETY: `l` comes from `allowed`, a subset of this node's mask.
                let cn = unsafe { self.cstep(col, l) };
                if base & cn.mask == 0 {
                    continue; // next row can't fit the columns
                }
                if pen {
                    // exact: the last rows are the completions of S by the letters column c allows
                    total += s.iter().map(|n| (n.mask & cn.mask).count_ones() as u64).sum::<u64>();
                    continue;
                }
                colv[c] = cn;
                total += self.count::<ANY, P>(r + 1, 0, self.root, colv, nodes, !0, std::slice::from_ref(&self.root), SELF);
                if ANY && total > 0 {
                    break;
                }
            }
            colv[c] = col;
            return total;
        }
        while good != 0 {
            let l = good.trailing_zeros();
            good &= good - 1;
            // SAFETY: `l` comes from `good`, a subset of this node's mask.
            let cn = unsafe { self.cstep(col, l) };
            let fc = base & cn.mask;
            if fc == 0 {
                continue; // next row can't fit the columns
            }
            colv[c] = cn;
            // SAFETY: `l` comes from `good`, a subset of this node's mask.
            total += self.count::<ANY, P>(r, c + 1, unsafe { self.rstep(row, l) }, colv, nodes, fc, s, cn.mask);
            if ANY && total > 0 {
                break;
            }
        }
        colv[c] = col;
        total
    }

    /// `count` specialized for the second-to-last row with a known next-row set S (here: the
    /// possible last rows). No bigram bound, no row change: the last cell sums the completions of
    /// S directly, and the last two cells are fused (no call, no set buffer for the last cell).
    /// Falls back to the generic `count` if S overflows `S_CAP`.
    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    fn count_pen<const ANY: bool, const P: bool>(&self, c: usize, row: Node, colv: &mut [CNode; W], nodes: &mut u64, f: u32, ps: &[Node], pm: u32) -> u64 {
        *nodes += 1;
        crate::probe::hit(self.last_r - 1, c);
        if ANY {
            if *nodes & 1023 == 0
                && let Some((bound, j, deadline)) = self.cut
                && (bound.load(Relaxed) <= j || deadline.is_some_and(|(d, hurry)| hurry.load(Relaxed) && Instant::now() > d))
            {
                self.cancelled.set(true);
            }
            if self.cancelled.get() {
                return 0;
            }
        }
        let col = colv[c];
        let allowed = row.mask & col.mask;
        let mut good = if c == (W - 1) {
            allowed
        } else {
            let m = |k: usize| if k < W { colv[k].mask } else { !0 };
            self.good(row, allowed, c, m(c + 1), m(c + 2), m(c + 3))
        };
        if good == 0 {
            return 0;
        }
        // materialize S
        let mut buf = SetBuf::<S_CAP_PEN>::new();
        let (s, base): (&[Node], u32) = if pm == SELF {
            (ps, ps.iter().fold(0, |a, n| a | n.mask))
        } else {
            let mut n = 0;
            let mut base = 0;
            for sn in ps {
                let mut m = sn.mask & pm;
                while m != 0 {
                    let x = m.trailing_zeros();
                    m &= m - 1;
                    if n == S_CAP_PEN {
                        return self.count::<ANY, P>(self.last_r - 1, c, row, colv, nodes, f, &[], SELF);
                    }
                    // SAFETY: `x` is a bit of `sn.mask`, `sn` a row-trie node.
                    let ch = unsafe { self.rstep(*sn, x) };
                    base |= ch.mask;
                    buf.set(n, ch);
                    n += 1;
                }
            }
            (buf.first(n), base)
        };
        let mut total = 0;
        if c == (W - 1) {
            while good != 0 {
                let l = good.trailing_zeros();
                good &= good - 1;
                // SAFETY: `l` comes from `allowed`, a subset of this node's mask.
                let m = unsafe { self.cstep(col, l) }.mask;
                total += s.iter().map(|n| (n.mask & m).count_ones() as u64).sum::<u64>();
            }
            return total;
        }
        if c + 2 == W {
            // fused last two cells: for letter l here and x in the last cell, the last rows are the
            // children y (allowed by column c after l) of S whose masks meet column W-1 after x
            let col2 = colv[W - 1];
            while good != 0 {
                let l = good.trailing_zeros();
                good &= good - 1;
                // SAFETY: `l` comes from `good`, a subset of this node's mask.
                let m = unsafe { self.cstep(col, l) }.mask;
                if base & m == 0 {
                    continue;
                }
                // SAFETY: as above.
                let child = unsafe { self.rstep(row, l) };
                let mut a2 = child.mask & col2.mask;
                if a2 == 0 {
                    continue;
                }
                *nodes += 1;
                // last-row letters at W-1 the column allows, given each letter x of this row
                let mut ends = [0u32; 26];
                let mut ne = 0;
                while a2 != 0 {
                    let x = a2.trailing_zeros();
                    a2 &= a2 - 1;
                    // SAFETY: `x` is a bit of `col2.mask`.
                    ends[ne] = unsafe { self.cstep(col2, x) }.mask;
                    ne += 1;
                }
                for sn in s {
                    let mut ym = sn.mask & m;
                    while ym != 0 {
                        let y = ym.trailing_zeros();
                        ym &= ym - 1;
                        // SAFETY: `y` is a bit of `sn.mask`.
                        let last = unsafe { self.rstep(*sn, y) }.mask;
                        total += ends[..ne].iter().map(|&e| (last & e).count_ones() as u64).sum::<u64>();
                    }
                }
                if ANY && total > 0 {
                    break;
                }
            }
            return total;
        }
        while good != 0 {
            let l = good.trailing_zeros();
            good &= good - 1;
            // SAFETY: `l` comes from `good`, a subset of this node's mask.
            let cn = unsafe { self.cstep(col, l) };
            let fc = base & cn.mask;
            if fc == 0 {
                continue; // no last row fits the columns
            }
            colv[c] = cn;
            // SAFETY: as above.
            total += self.count_pen::<ANY, P>(c + 1, unsafe { self.rstep(row, l) }, colv, nodes, fc, s, cn.mask);
            if ANY && total > 0 {
                break;
            }
        }
        colv[c] = col;
        total
    }

    /// Letters a row word can have at position k + 1, given its letter at k is in `f`.
    #[inline(always)]
    fn uni(&self, k: usize, f: u32) -> u32 {
        let t = &self.bi[k];
        t[0][(f & 127) as usize] | t[1][((f >> 7) & 127) as usize] | t[2][((f >> 14) & 127) as usize] | t[3][((f >> 21) & 31) as usize]
    }

    /// Branch-free look-ahead filter: the letters of `allowed` whose row child can still match
    /// the columns at c+1, c+2, c+3 (masks `m1..m3`, `!0` past the end). Doing it in a separate
    /// pass turns one unpredictable branch per letter into straight-line code.
    #[inline(always)]
    fn good(&self, row: Node, allowed: u32, c: usize, m1: u32, m2: u32, m3: u32) -> u32 {
        let (e2, e3) = (c + 2 >= W, c + 3 >= W);
        let mut a = allowed;
        let mut good = 0;
        while a != 0 {
            let l = a.trailing_zeros();
            a &= a - 1;
            // SAFETY: `l` comes from `allowed`/`good`, subsets of this node's mask.
            let n = unsafe { self.rstep(row, l) };
            let ok = (n.mask & m1 != 0) & (e2 | (n.next & m2 != 0)) & (e3 | (n.next2 & m3 != 0));
            good |= (ok as u32) << l;
        }
        good
    }
}

/// Stack buffer for a next-row set, left uninitialized: zero-filling it cost a 256–512 byte memset
/// at every node.
struct SetBuf<const N: usize> {
    n: [std::mem::MaybeUninit<Node>; N],
}

impl<const N: usize> SetBuf<N> {
    #[inline(always)]
    fn new() -> Self {
        SetBuf { n: [std::mem::MaybeUninit::uninit(); N] }
    }

    /// Write slot `i` (`i < N`, checked by the callers' `n == CAP` tests).
    #[inline(always)]
    fn set(&mut self, i: usize, v: Node) {
        self.n[i].write(v);
    }

    /// The first `len` slots, all written by `set`.
    #[inline(always)]
    fn first(&self, len: usize) -> &[Node] {
        debug_assert!(len <= N);
        // SAFETY: callers wrote slots 0..len with `set` before (fill loops write 0, 1, 2, …),
        // and MaybeUninit<Node> has Node's layout.
        unsafe { std::slice::from_raw_parts(self.n.as_ptr().cast::<Node>(), len) }
    }
}

/// Successor table for one position k: `t[chunk][v]` = letters that follow (at k + 1) some letter of
/// the 7-bit chunk `v` (letters 7·chunk ..) at k. The union over any letter set `f` is 4 lookups.
type Bigram = [[u32; 128]; 4];

fn bigrams(words: &[String], len: usize) -> Vec<Bigram> {
    let mut t = vec![[[0u32; 128]; 4]; len.max(1)];
    for w in words {
        let b = w.as_bytes();
        for k in 0..len - 1 {
            let (a, n) = ((b[k] - b'a') as usize, (b[k + 1] - b'a') as u32);
            let (chunk, bit) = (a / 7, a % 7);
            for (v, e) in t[k][chunk].iter_mut().enumerate() {
                if v >> bit & 1 == 1 {
                    *e |= 1 << n;
                }
            }
        }
    }
    t
}

impl ParSearch {
    /// `threads == 0` = all available cores.
    pub fn new(rows: usize, cols: usize, row_words: &[String], col_words: &[String], threads: usize) -> Self {
        assert!(cols <= 32, "par supports at most 32 columns (last-row mask array)");
        let threads = if threads == 0 {
            std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
        } else {
            threads
        };
        ParSearch {
            rows,
            cols,
            row_words: row_words.to_vec(),
            col_words: col_words.to_vec(),
            mirrored: false,
            anti: false,
            sync_mirror: false,
            eager_split: false,
            row_trie: Trie::new(row_words),
            col_trie: CTrie::new(&Trie::new(col_words)),
            threads,
            seq_budget: FIND_SEQ_BUDGET,
            sym: rows == cols && row_words == col_words,
            bigrams: bigrams(row_words, cols),
            stats: Stats::default(),
        }
    }

    /// `find` builds its mirror image before starting the parallel jobs, so every big-grid job is
    /// tested on it (tests use this; see `find`).
    pub fn with_sync_mirror(mut self) -> Self {
        self.sync_mirror = true;
        self
    }

    /// `find` splits every job into sub-jobs, recursively (tests the split path; see `find`).
    pub fn with_eager_split(mut self) -> Self {
        self.eager_split = true;
        self
    }

    /// Node budget of `find`'s sequential first attempt (0 = always go parallel; for tests).
    pub fn with_seq_budget(mut self, budget: u64) -> Self {
        self.seq_budget = budget;
        self
    }

    /// Breadth-first expansion of the first cells into lexicographically ordered jobs.
    /// Never expands the last cell, so every job still has at least one cell to fill.
    fn jobs(&mut self, per_thread: usize, max_depth: usize) -> (usize, Vec<u8>) {
        let n = self.rows * self.cols;
        let target = per_thread * self.threads;
        let mut w = Worker::new(self);
        let mut depth = 0;
        let mut jobs: Vec<u8> = Vec::new(); // flattened prefixes of length `depth`
        let mut count = 1;
        let mut expanded = 0;
        while count < target && depth + 1 < n && depth < max_depth {
            let mut next = Vec::new();
            let mut next_count = 0;
            for j in 0..count {
                let prefix = &jobs[j * depth..(j + 1) * depth];
                w.load(prefix);
                expanded += 1;
                let (_, _, _, _, mut allowed) = w.allowed(depth);
                while allowed != 0 {
                    let l = allowed.trailing_zeros() as u8;
                    allowed &= allowed - 1;
                    next.extend_from_slice(prefix);
                    next.push(l);
                    next_count += 1;
                }
            }
            jobs = next;
            count = next_count;
            depth += 1;
            if count == 0 {
                break;
            }
        }
        self.stats.nodes += expanded;
        (depth, jobs)
    }

    /// Run `work(worker, job_index, prefix)` over all jobs on all threads, in rough job order.
    fn run<T: Send>(&mut self, depth: usize, jobs: &[u8], work: impl Fn(&mut Worker, usize, &[u8]) -> Option<T> + Sync) -> Vec<(usize, T)> {
        let count = jobs.len().checked_div(depth).unwrap_or(1);
        let next = AtomicUsize::new(0);
        let nodes = AtomicU64::new(0);
        let results = Mutex::new(Vec::new());
        let this: &ParSearch = self;
        std::thread::scope(|scope| {
            for _ in 0..this.threads.min(count) {
                scope.spawn(|| {
                    let mut w = Worker::new(this);
                    let mut mine = Vec::new();
                    loop {
                        let j = next.fetch_add(1, Relaxed);
                        if j >= count {
                            break;
                        }
                        let prefix = &jobs[j * depth..(j + 1) * depth];
                        w.load(prefix);
                        if let Some(t) = work(&mut w, j, prefix) {
                            mine.push((j, t));
                        }
                    }
                    nodes.fetch_add(w.nodes, Relaxed);
                    crate::probe::flush();
                    results.lock().unwrap().extend(mine);
                });
            }
        });
        self.stats.nodes += nodes.into_inner();
        let mut r = results.into_inner().unwrap();
        r.sort_by_key(|(j, _)| *j);
        r
    }

    /// On a mirrored search (row words reversed): is there a grid whose *original* first cells
    /// (row-major) are `prefix` (letters 0..26)? `None` = cut short (see `Cut`). In the mirror image,
    /// the prefix's full rows are placed directly (only the columns need them) and the letters of
    /// its partial row q are the end of row q, right to left. Those are forced by starting their
    /// columns at virtual nodes `{mask: 1 << l, first: index of the real child}`: the only letter
    /// allowed is `l`, and stepping it lands on the real column-trie child. No kernel change needed.
    fn exists_with_prefix(&self, prefix: &[u8], cut: Cut, nodes: &mut u64) -> Option<bool> {
        by_width!(self.cols, |W| self.exists_w::<W>(prefix, cut, nodes))
    }

    fn exists_w<const W: usize>(&self, prefix: &[u8], cut: Cut, nodes: &mut u64) -> Option<bool> {
        let (q, k) = (prefix.len() / W, prefix.len() % W);
        let mut col = [0u32; W]; // column-trie node indices after the full rows
        for (cell, &l) in prefix[..q * W].iter().enumerate() {
            let c = W - 1 - cell % W;
            if self.col_trie.mask(col[c]) >> l & 1 == 0 {
                return Some(false);
            }
            col[c] = self.col_trie.child(col[c], l as usize);
        }
        let mut colv: [CNode; W] = std::array::from_fn(|c| self.col_trie.node(col[c]));
        for (i, &l) in prefix[q * W..].iter().enumerate() {
            let c = W - 1 - i;
            if colv[c].mask >> l & 1 == 0 {
                return Some(false);
            }
            colv[c] = CNode { mask: 1 << l, first: self.col_trie.child(col[c], l as usize) };
        }
        debug_assert!(q < self.rows && (q + 1 < self.rows || k < W));
        let mut kn = Kernel::<W>::new(self);
        kn.cut = Some(cut);
        let row = kn.root;
        let ps = std::slice::from_ref(&kn.root);
        let total = if Kernel::<W>::PEN && self.rows <= 4 {
            kn.count::<true, true>(q, 0, row, &mut colv, nodes, !0, ps, SELF)
        } else {
            kn.count::<true, false>(q, 0, row, &mut colv, nodes, !0, ps, SELF)
        };
        if total > 0 { Some(true) } else if kn.cancelled.get() { None } else { Some(false) }
    }

    /// Breadth-first split of one job prefix into at least `target` sub-jobs (fewer if the tree is
    /// small), in lexicographic order. Never expands the last cell. Also returns the nodes expanded.
    fn expand(&self, w: &mut Worker, prefix: &[u8], target: usize) -> (Vec<Vec<u8>>, u64) {
        let n = self.rows * self.cols;
        let mut jobs = vec![prefix.to_vec()];
        let mut expanded = 0;
        while jobs.len() < target && jobs[0].len() + 1 < n {
            let mut next = Vec::new();
            for p in &jobs {
                w.load(p);
                expanded += 1;
                let (_, _, _, _, mut allowed) = w.allowed(p.len());
                while allowed != 0 {
                    let l = allowed.trailing_zeros() as u8;
                    allowed &= allowed - 1;
                    let mut q = p.clone();
                    q.push(l);
                    next.push(q);
                }
            }
            if next.is_empty() {
                break;
            }
            jobs = next;
        }
        (jobs, expanded)
    }

    fn to_grid(&self, g: &[u8]) -> Grid {
        g.chunks(self.cols).map(|r| String::from_utf8(r.to_vec()).unwrap()).collect()
    }
}

impl Solver for ParSearch {
    fn find(&mut self) -> Option<Grid> {
        // 1. short sequential attempt (same lexicographic order, so same answer)
        let mut w = Worker::new(self);
        w.limit = self.seq_budget;
        let mut sol = None;
        if self.seq_budget > 0 {
            w.search(0, &mut |g: &[u8]| {
                sol = Some(g.to_vec());
                false
            }, &|| false);
        }
        let (nodes, out_of_budget) = (w.nodes, self.seq_budget == 0 || w.nodes > w.limit);
        self.stats.nodes += nodes;
        if !out_of_budget || sol.is_some() {
            let g = sol.map(|g| self.to_grid(&g));
            self.stats.solutions = g.is_some() as u64;
            return g;
        }
        // 2. parallel: restart from scratch on many small jobs. On big grids (area >= 48, rows of
        //    8+ letters) jobs stay inside row 0, so the mirror test below applies. Big grids rarely
        //    have solutions, so proving jobs empty dominates. On smaller ones solutions come early,
        //    and row-0 jobs are too coarse: one thread walks the job holding the first solution
        //    (5×8: 25 -> 100 ms, 5×9: 150 -> 187 ms with the cap).
        let use_mirror = self.cols >= 8 && self.rows * self.cols >= 48;
        let (depth, jobs) = self.jobs(FIND_JOBS_PER_THREAD, if use_mirror { self.cols } else { usize::MAX });
        // 3. each job is first *tested*: does any grid extend its prefix? Only a job with solutions
        //    is searched (from its start, so it still yields its first grid). The test runs the count
        //    kernel, and once it is built (on a detached thread: it takes tens of ms, and a short find
        //    must not wait for it), on the mirror image (rows reversed: 1.4–1.7× faster on exhaustive
        //    work). There the job's prefix is the *end* of row 0, forced through virtual column nodes
        //    (`exists_with_prefix`).
        let mirror: std::sync::Arc<std::sync::OnceLock<ParSearch>> = Default::default();
        if use_mirror && depth <= self.cols {
            let (m, rw, cw) = (mirror.clone(), self.row_words.clone(), self.col_words.clone());
            let (rows, cols, threads) = (self.rows, self.cols, self.threads);
            let build = move || {
                let rev: Vec<String> = rw.iter().map(|w| w.chars().rev().collect()).collect();
                let _ = m.set(ParSearch::new(rows, cols, &rev, &cw, threads));
            };
            if self.sync_mirror {
                build();
            } else {
                std::thread::spawn(build);
            }
        }
        // 4. every job gets a time budget (test, then search; enforced only while a thread is idle). Work over
        //    budget is split (budget doubled), and the pieces go to a shared queue that workers serve
        //    first, so all threads help on a huge subtree instead of one walking it while the others
        //    wait (7×7: 104 -> 62 ms, 6×7: 82 -> 43 ms). Jobs are ranked by disjoint intervals
        //    [lo, hi) in lexicographic order (top job j: [j·SPAN, (j+1)·SPAN), sub-jobs split their
        //    parent's interval), so the solution of the lowest-ranked job that has one is the first
        //    grid, and work ranked above a found solution is skipped.
        //    A mirror test is split in the *mirror's* order (`Part`s: more of the mirror's rows,
        //    left to right), since forcing original-order letters at the end of mirror rows would
        //    redo most of the parent's test in every sub-job (6×9: 6.0 -> 12.2 s). The parts only
        //    decide whether the job has a solution; the first part that finds one starts the job's
        //    search.
        const SPAN: u64 = 1 << 40;
        let count = jobs.len().checked_div(depth).unwrap_or(1);
        let budget = Duration::from_micros(if self.eager_split { 0 } else { FIND_JOB_BUDGET_US });
        // `bound`: everything ranked at or above it is useless, because an earlier interval is known
        // to hold a solution (min `hi` over jobs whose test or search found one)
        let bound = AtomicU64::new(u64::MAX);
        // set while some thread is idle and the queue is empty: only then is work split (aborting a
        // job wastes what it did, which only pays if a thread would otherwise wait)
        let hurry = AtomicBool::new(self.eager_split);
        let (next, inflight) = (AtomicUsize::new(0), AtomicUsize::new(self.threads.min(count)));
        let queue = Mutex::new(std::collections::BinaryHeap::<Task>::new());
        let queued = AtomicUsize::new(0);
        let (nodes, extra) = (AtomicU64::new(0), AtomicU64::new(0));
        let results = Mutex::new(Vec::new());
        let this: &ParSearch = self;
        let w_cols = self.cols;
        let push = |tasks: Vec<Task>| {
            let mut q = queue.lock().unwrap();
            q.extend(tasks);
            queued.store(q.len(), SeqCst);
        };
        // `subs` (lexicographic order) as tasks with sub-intervals of [lo, hi); None if they can't be ranked
        let ranked = |lo: u64, hi: u64, budget: Duration, subs: Vec<Vec<u8>>| -> Option<Vec<Task>> {
            let step = (hi - lo) / subs.len() as u64;
            if step == 0 || subs.len() < 2 {
                return None;
            }
            let last = subs.len() - 1;
            Some(subs.into_iter().enumerate().map(|(i, prefix)| {
                let a = lo + i as u64 * step;
                Task { lo: a, hi: if i == last { hi } else { a + step }, budget: budget * 2, kind: TaskKind::Job { prefix, mirror_ok: false } }
            }).collect())
        };
        let deadline = |split: bool, budget: Duration| split.then(|| (Instant::now() + budget, &hurry));
        // probe builds only: timeline events, and WR_NO_SPLIT=1 turns splitting off (for the chart)
        let no_split = crate::probe::ENABLED && std::env::var_os("WR_NO_SPLIT").is_some();
        let clock = || crate::probe::ENABLED.then(Instant::now);
        let ev = |kind: u8, t: Option<Instant>, lo: u64| {
            if let Some(t) = t {
                crate::probe::event(kind, t, lo);
            }
        };
        crate::probe::epoch();
        // the job is known to have solutions: find its first one
        let search = |w: &mut Worker, lo: u64, hi: u64, prefix: &[u8], budget: Duration| {
            let mut split = !no_split;
            loop {
                let t = clock();
                let d = deadline(split, budget);
                let late = || d.is_some_and(|(d, h)| h.load(Relaxed) && Instant::now() > d);
                let mut sol = None;
                w.load(prefix);
                w.search(prefix.len(), &mut |g: &[u8]| {
                    sol = Some(g.to_vec());
                    false
                }, &|| bound.load(Relaxed) <= lo || late());
                if let Some(g) = sol {
                    bound.fetch_min(hi, Relaxed);
                    results.lock().unwrap().push((lo, g));
                    ev(3, t, lo);
                    return;
                }
                if bound.load(Relaxed) <= lo {
                    ev(5, t, lo);
                    return;
                }
                ev(2, t, lo);
                let (subs, n) = this.expand(w, prefix, 4 * this.threads);
                extra.fetch_add(n, Relaxed);
                match ranked(lo, hi, budget, subs) {
                    Some(t) => return push(t),
                    None => split = false, // can't split (rank space or tree exhausted): finish here
                }
            }
        };
        // tri-state test result: Some(any) or None = cut short
        let tri = |any: bool, cancelled: bool| if any { Some(true) } else if cancelled { None } else { Some(false) };
        let job = |w: &mut Worker, mut mw: Option<&mut Worker>, lo: u64, hi: u64, prefix: &[u8], budget: Duration, mirror_ok: bool| {
            if bound.load(Relaxed) <= lo {
                return; // an earlier job already has a solution
            }
            let mut split = !no_split;
            loop {
                let t = clock();
                let cut = (&bound, lo, deadline(split, budget));
                let eager = split && this.eager_split; // tests: split without trying
                let any = match mw.as_deref_mut() {
                    Some(mw) if mirror_ok && !prefix.len().is_multiple_of(w_cols) => {
                        // partial row: forced through virtual nodes, never split
                        let mut n = 0;
                        let any = mw.s.exists_with_prefix(prefix, (&bound, lo, None), &mut n);
                        extra.fetch_add(n, Relaxed);
                        any
                    }
                    Some(mw) if mirror_ok => {
                        // full rows: test on the mirror's own worker (each row reversed)
                        let m: Vec<u8> = prefix.chunks(w_cols).flat_map(|r| r.iter().rev().copied()).collect();
                        let any = if eager { None } else {
                            mw.load(&m);
                            let any = mw.count_job(m.len(), Some(cut)) > 0;
                            tri(any, mw.cancelled)
                        };
                        if any.is_none() && split && bound.load(Relaxed) > lo {
                            ev(2, t, lo);
                            let (parts, n) = mw.s.expand(mw, &m, 4 * this.threads);
                            extra.fetch_add(n, Relaxed);
                            if parts.len() >= 2 {
                                let group = std::sync::Arc::new(AtomicBool::new(false));
                                return push(parts.into_iter().map(|m| Task { lo, hi, budget: budget * 2, kind: TaskKind::Part { group: group.clone(), prefix: prefix.to_vec(), m } }).collect());
                            }
                            split = false;
                            continue;
                        }
                        any
                    }
                    _ => {
                        if eager { None } else {
                            w.load(prefix);
                            let any = w.count_job(prefix.len(), Some(cut)) > 0;
                            tri(any, w.cancelled)
                        }
                    }
                };
                if bound.load(Relaxed) <= lo || any == Some(false) {
                    ev(if any == Some(false) { 0 } else { 5 }, t, lo);
                    return;
                }
                ev(if any == Some(true) { 1 } else { 2 }, t, lo);
                if any == Some(true) {
                    // the first grid is in this job: later jobs are useless
                    bound.fetch_min(hi, Relaxed);
                    return search(w, lo, hi, prefix, budget);
                }
                // over budget: split into sub-jobs
                let (subs, n) = this.expand(w, prefix, 4 * this.threads);
                extra.fetch_add(n, Relaxed);
                match ranked(lo, hi, budget, subs) {
                    Some(t) => return push(t),
                    None => split = false,
                }
            }
        };
        // one part of a job's mirror test
        let part = |w: &mut Worker, mw: &mut Worker, t: &Task, group: &AtomicBool, prefix: &[u8], m: &[u8]| {
            let mut split = !no_split;
            loop {
                let tt = clock();
                if bound.load(Relaxed) <= t.lo || group.load(Relaxed) {
                    return; // the job is settled
                }
                let any = if split && this.eager_split { None } else {
                    mw.load(m);
                    let any = mw.count_job(m.len(), Some((&bound, t.lo, deadline(split, t.budget)))) > 0;
                    tri(any, mw.cancelled)
                };
                ev(if any.is_none() { 2 } else { 4 }, tt, t.lo);
                match any {
                    Some(false) => return,
                    Some(true) => {
                        if !group.swap(true, Relaxed) {
                            bound.fetch_min(t.hi, Relaxed);
                            search(w, t.lo, t.hi, prefix, t.budget);
                        }
                        return;
                    }
                    None if bound.load(Relaxed) <= t.lo => return,
                    None => {
                        let (parts, n) = mw.s.expand(mw, m, 4 * this.threads);
                        extra.fetch_add(n, Relaxed);
                        if parts.len() < 2 {
                            split = false;
                            continue;
                        }
                        let tasks = parts.into_iter().map(|m| Task { lo: t.lo, hi: t.hi, budget: t.budget * 2, kind: TaskKind::Part { group: match &t.kind { TaskKind::Part { group, .. } => group.clone(), _ => unreachable!() }, prefix: prefix.to_vec(), m } }).collect();
                        return push(tasks);
                    }
                }
            }
        };
        std::thread::scope(|scope| {
            for _ in 0..this.threads.min(count) {
                scope.spawn(|| {
                    let mut w = Worker::new(this);
                    let mut mw: Option<Worker> = None;
                    // `inflight` counts the threads that are not idle (only they add to the queue)
                    'work: loop {
                        if mw.is_none() && let Some(m) = mirror.get() {
                            mw = Some(Worker::new(m));
                        }
                        let urgent = if queued.load(Relaxed) > 0 {
                            let mut q = queue.lock().unwrap();
                            let t = q.pop();
                            queued.store(q.len(), SeqCst);
                            t
                        } else {
                            None
                        };
                        if let Some(t) = urgent {
                            match &t.kind {
                                TaskKind::Job { prefix, mirror_ok } => job(&mut w, mw.as_mut(), t.lo, t.hi, prefix, t.budget, *mirror_ok),
                                TaskKind::Part { group, prefix, m } => part(&mut w, mw.as_mut().expect("parts exist only once the mirror does"), &t, group, prefix, m),
                            }
                            continue;
                        }
                        let j = next.fetch_add(1, Relaxed);
                        if j < count {
                            let prefix = &jobs[j * depth..(j + 1) * depth];
                            job(&mut w, mw.as_mut(), j as u64 * SPAN, (j as u64 + 1) * SPAN, prefix, budget, true);
                            continue;
                        }
                        inflight.fetch_sub(1, SeqCst);
                        loop {
                            if queued.load(SeqCst) > 0 {
                                if inflight.fetch_add(1, SeqCst) + 1 == this.threads.min(count) {
                                    hurry.store(this.eager_split, Relaxed); // nobody idle any more
                                }
                                continue 'work;
                            }
                            hurry.store(true, Relaxed);
                            if inflight.load(SeqCst) == 0 && queued.load(SeqCst) == 0 {
                                break 'work;
                            }
                            std::thread::yield_now();
                        }
                    }
                    nodes.fetch_add(w.nodes + mw.map_or(0, |m| m.nodes), Relaxed);
                    crate::probe::flush();
                });
            }
        });
        self.stats.nodes += nodes.into_inner() + extra.into_inner();
        let answer = results.into_inner().unwrap().into_iter().min_by_key(|(lo, _)| *lo).map(|(_, g)| g);
        let g = answer.map(|g| self.to_grid(&g));
        self.stats.solutions = g.is_some() as u64;
        g
    }
    fn count(&mut self) -> u64 {
        // Counts don't depend on orientation, so count the cheapest one:
        // 1. tall grids transposed: long rows are much cheaper (6×4 explores 2.4× the nodes of 4×6);
        // 2. rows mirrored (each row word reversed, so rows fill from their last letter): 1.07–1.6×
        //    faster on every shape measured. A square with one word list loses its transpose
        //    symmetry when mirrored, but gains an anti-diagonal one (`Kernel::count_anti`): faster
        //    from 6×6 up (1.13× at 6×6, 1.34× at 7×7, 1.50× at 8×8), slightly slower below.
        let other = if self.rows > self.cols && self.rows <= 32 {
            Some(ParSearch::new(self.cols, self.rows, &self.col_words, &self.row_words, self.threads))
        } else if !self.mirrored && (!self.sym || self.cols >= 6) {
            let rev: Vec<String> = self.row_words.iter().map(|w| w.chars().rev().collect()).collect();
            // 3. without a symmetry to keep, also swap the first two rows (each column word's first two
            //    letters): of all 24 row orders of 4×5, 4×6 and 4×7 this is the fastest, 0.5–4% faster
            //    than top-down (3×7 5%, 5×6 0.3%)
            let cols: Vec<String> = if self.sym || self.rows < 3 {
                self.col_words.clone()
            } else {
                self.col_words.iter().map(|w| {
                    let mut b = w.clone().into_bytes();
                    b.swap(0, 1);
                    String::from_utf8(b).unwrap()
                }).collect()
            };
            let mut m = ParSearch::new(self.rows, self.cols, &rev, &cols, self.threads);
            m.mirrored = true;
            m.anti = self.sym; // the mirror of a square keeps a symmetry: the anti-diagonal one
            Some(m)
        } else {
            None
        };
        if let Some(mut t) = other {
            let total = t.count();
            self.stats = t.stats;
            return total;
        }
        // with symmetry, job prefixes must stay in row 0 (they are expanded without the tie rule)
        let max_depth = if self.sym || self.anti { self.cols } else { usize::MAX };
        let (depth, jobs) = self.jobs(COUNT_JOBS_PER_THREAD, max_depth);
        let total: u64 = self.run(depth, &jobs, |w, _, _| Some(w.count_job(depth, None))).iter().map(|(_, c)| c).sum();
        self.stats.solutions = total;
        total
    }

    fn all(&mut self) -> Vec<Grid> {
        let (depth, jobs) = self.jobs(COUNT_JOBS_PER_THREAD, usize::MAX);
        let per_job = self.run(depth, &jobs, |w, _, _| {
            let mut v = Vec::new();
            w.search(depth, &mut |g: &[u8]| {
                v.push(g.to_vec());
                true
            }, &|| false);
            Some(v)
        });
        let out: Vec<Grid> = per_job.into_iter().flat_map(|(_, v)| v).map(|g| self.to_grid(&g)).collect();
        self.stats.solutions = out.len() as u64;
        out
    }

    fn stats(&self) -> Stats {
        self.stats
    }
}
