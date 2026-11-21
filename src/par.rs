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
//! 3. `find` is deterministic: `best` = lowest job index that found a solution; workers
//!    skip / abort any job after it, so the answer is the lexicographically first grid,
//!    exactly what the sequential solver returns.

use crate::trie::{CNode, CTrie, Node, Trie};

// Layout guard (rust-skills `mem-assert-type-size`): the kernels are tuned for these sizes.
const _: () = assert!(size_of::<Node>() == 16 && size_of::<CNode>() == 8);
use crate::{Grid, Solver, Stats};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};

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

/// Jobs per thread. Exhaustive counting balances fine with few jobs; `find` needs many
/// small jobs, because the job that holds the first solution must not be one huge subtree.
const COUNT_JOBS_PER_THREAD: usize = 256;
const FIND_JOBS_PER_THREAD: usize = 4096;
/// `find` first searches sequentially for this many nodes (~10 ms); small instances finish
/// there without paying for threads and job expansion.
const FIND_SEQ_BUDGET: u64 = 2_000_000;
/// Largest exact next-row set the count kernel tracks (see `Kernel::count`).
const S_CAP: usize = 16;
/// Same, in the second-to-last row (`Kernel::count_pen`), where overflowing is costlier.
const S_CAP_PEN: usize = 32;
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
    /// Tall grids (rows > cols): the transposed search, used by `count`. Long rows are much
    /// cheaper to count (6×4 explores 2.4× the nodes of 4×6). `find` keeps the original
    /// orientation, because its answer must be the row-major lexicographically first grid.
    flip: Option<Box<ParSearch>>,
    stats: Stats,
}

/// Per-thread search state.
struct Worker<'a> {
    s: &'a ParSearch,
    row_state: Vec<u32>,
    col_state: Vec<u32>,
    grid: Vec<u8>,
    nodes: u64,
    /// `search` gives up once `nodes` exceeds this (checked every 256 nodes, with `stop`).
    limit: u64,
}

impl<'a> Worker<'a> {
    fn new(s: &'a ParSearch) -> Self {
        Worker { s, row_state: vec![0; s.rows], col_state: vec![0; s.cols], grid: vec![0; s.rows * s.cols], nodes: 0, limit: u64::MAX }
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
    /// `plain` = Some((best, job)) for `find`: no square symmetry (prefixes leave row 0), stop at the
    /// first solution (the result is then only "0" or "some"), and give up (0) once `best` < job.
    fn count_job(&mut self, cell: usize, plain: Option<(&AtomicUsize, usize)>) -> u64 {
        by_width!(self.s.cols, |W| self.count_job_w::<W>(cell, plain))
    }

    fn count_job_w<const W: usize>(&mut self, cell: usize, plain: Option<(&AtomicUsize, usize)>) -> u64 {
        let s = self.s;
        let (r, c) = (cell / W, cell % W);
        let mut colv: [CNode; W] = std::array::from_fn(|i| s.col_trie.node(self.col_state[i]));
        let mut k = Kernel::<W>::new(s);
        k.cut = plain;
        let mut nodes = 0;
        let row = s.row_trie.node(self.row_state[r]);
        let total = if s.sym && plain.is_none() {
            // job prefixes stay inside row 0 when `sym` (see `count`), so they are all "tied"
            let mut row0 = [0u8; W];
            for (d, &g) in row0.iter_mut().zip(&self.grid[..cell]) {
                *d = g - b'a';
            }
            k.count_sym(r, c, row, &mut colv, &mut nodes, &mut row0)
        } else {
            let ps: &[Node] = if c == 0 { std::slice::from_ref(&k.root) } else { &[] };
            if plain.is_some() { k.count::<true>(r, c, row, &mut colv, &mut nodes, !0, ps, SELF) } else { k.count::<false>(r, c, row, &mut colv, &mut nodes, !0, ps, SELF) }
        };
        self.nodes += nodes;
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
    /// `count::<ANY>` only: (best job so far, this job). The job is cancelled once an earlier job
    /// has a solution; `cancelled` then makes every frame return at once.
    cut: Option<(&'t AtomicUsize, usize)>,
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
    /// Track the exact next-row set only on rows of up to 8 letters: on longer rows it overflows
    /// `S_CAP` almost at once and only costs time (6×9 count: 10.5 s with it, 9.4 s without).
    const TRACK: bool = W <= 8;

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
    fn count_sym(&self, r: usize, c: usize, row: Node, colv: &mut [CNode; W], nodes: &mut u64, row0: &mut [u8; W]) -> u64 {
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
                2 * self.count::<false>(nr, nc, nrow, colv, nodes, !0, if nc == 0 { std::slice::from_ref(&self.root) } else { &[] }, SELF)
            } else {
                self.count_sym(nr, nc, nrow, colv, nodes, row0)
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
    /// `ps`, `pm` = the exact set S of row-trie nodes (depth c) that row r + 1 can still be in, given
    /// the column letters already forced on its positions 0..c-1: S = children of the nodes `ps` by
    /// letters in `pm`, or `ps` itself when `pm == SELF`. Built lazily, only if this cell has a live
    /// letter. `ps` empty = unknown (S had more than `S_CAP` nodes): the bigram bound `f` is used.
    #[allow(clippy::too_many_arguments)]
    fn count<const ANY: bool>(&self, r: usize, c: usize, row: Node, colv: &mut [CNode; W], nodes: &mut u64, f: u32, ps: &[Node], pm: u32) -> u64 {
        if r == self.last_r {
            // Last row: columns only need their masks (their children are never used).
            let mut masks = [0u32; W];
            for (m, n) in masks.iter_mut().zip(colv.iter()) {
                *m = n.mask;
            }
            return self.count_last(c, row, &masks, nodes);
        }
        if Self::TRACK && r + 1 == self.last_r && !ps.is_empty() {
            return self.count_pen::<ANY>(c, row, colv, nodes, f, ps, pm);
        }
        *nodes += 1;
        crate::probe::hit(r, c);
        if ANY {
            if *nodes & 1023 == 0
                && let Some((best, j)) = self.cut
                && best.load(Relaxed) < j
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
                total += self.count::<ANY>(r + 1, 0, self.root, colv, nodes, !0, std::slice::from_ref(&self.root), SELF);
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
            total += self.count::<ANY>(r, c + 1, unsafe { self.rstep(row, l) }, colv, nodes, fc, s, cn.mask);
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
    fn count_pen<const ANY: bool>(&self, c: usize, row: Node, colv: &mut [CNode; W], nodes: &mut u64, f: u32, ps: &[Node], pm: u32) -> u64 {
        *nodes += 1;
        crate::probe::hit(self.last_r - 1, c);
        if ANY {
            if *nodes & 1023 == 0
                && let Some((best, j)) = self.cut
                && best.load(Relaxed) < j
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
                        return self.count::<ANY>(self.last_r - 1, c, row, colv, nodes, f, &[], SELF);
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
            total += self.count_pen::<ANY>(c + 1, unsafe { self.rstep(row, l) }, colv, nodes, fc, s, cn.mask);
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
            for v in 0..128 {
                if v >> bit & 1 == 1 {
                    t[k][chunk][v] |= 1 << n;
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
        let flip = (rows > cols && rows <= 32).then(|| Box::new(ParSearch::new(cols, rows, col_words, row_words, threads)));
        ParSearch {
            rows,
            cols,
            flip,
            row_trie: Trie::new(row_words),
            col_trie: CTrie::new(&Trie::new(col_words)),
            threads,
            seq_budget: FIND_SEQ_BUDGET,
            sym: rows == cols && row_words == col_words,
            bigrams: bigrams(row_words, cols),
            stats: Stats::default(),
        }
    }

    /// Node budget of `find`'s sequential first attempt (0 = always go parallel; for tests).
    pub fn with_seq_budget(mut self, budget: u64) -> Self {
        self.seq_budget = budget;
        self.flip = self.flip.map(|t| Box::new(t.with_seq_budget(budget)));
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
        w.search(0, &mut |g: &[u8]| {
            sol = Some(g.to_vec());
            false
        }, &|| false);
        let (nodes, out_of_budget) = (w.nodes, w.nodes > w.limit);
        self.stats.nodes += nodes;
        if !out_of_budget || sol.is_some() {
            let g = sol.map(|g| self.to_grid(&g));
            self.stats.solutions = g.is_some() as u64;
            return g;
        }
        // 2. parallel: restart from scratch on many small jobs
        let (depth, jobs) = self.jobs(FIND_JOBS_PER_THREAD, usize::MAX);
        let best = AtomicUsize::new(usize::MAX);
        let found = self.run(depth, &jobs, |w, j, _| {
            if best.load(Relaxed) < j {
                return None; // an earlier job already has a solution
            }
            // the count kernel is faster than `search` and proves most jobs empty; only a job
            // with solutions is searched (from its start, so it still yields its first grid)
            if w.count_job(depth, Some((&best, j))) == 0 || best.load(Relaxed) < j {
                return None;
            }
            let mut sol = None;
            w.search(depth, &mut |g: &[u8]| {
                sol = Some(g.to_vec());
                false
            }, &|| best.load(Relaxed) < j);
            if sol.is_some() {
                best.fetch_min(j, Relaxed);
            }
            sol
        });
        let g = found.into_iter().next().map(|(_, g)| self.to_grid(&g));
        self.stats.solutions = g.is_some() as u64;
        g
    }

    fn count(&mut self) -> u64 {
        if let Some(t) = &mut self.flip {
            let total = t.count();
            self.stats = t.stats;
            return total;
        }
        // with symmetry, job prefixes must stay in row 0 (they are expanded without the tie rule)
        let max_depth = if self.sym { self.cols } else { usize::MAX };
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
