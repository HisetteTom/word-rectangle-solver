//! Level 6: parallel version of the row-major trie search (`search.rs`), std threads only.
//! The hot loops (`Kernel`) carry trie nodes by value and track (row, col) directly.
//! Square counts use transpose symmetry (`Kernel::count_sym`): about half the nodes.
//! Row look-ahead: a letter is skipped when the row node it leads to (`next`, `next2` masks)
//! can't fit the columns two and three cells further right (forward checking).
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
    fn count_job(&mut self, cell: usize) -> u64 {
        by_width!(self.s.cols, |W| self.count_job_w::<W>(cell))
    }

    fn count_job_w<const W: usize>(&mut self, cell: usize) -> u64 {
        let s = self.s;
        let (r, c) = (cell / W, cell % W);
        let mut colv: [CNode; W] = std::array::from_fn(|i| s.col_trie.node(self.col_state[i]));
        let k = Kernel::<W> { rt: &s.row_trie, bi: &s.bigrams, ct: &s.col_trie, last_r: s.rows - 1 };
        let mut nodes = 0;
        let row = s.row_trie.node(self.row_state[r]);
        let total = if s.sym {
            // job prefixes stay inside row 0 when `sym` (see `count`), so they are all "tied"
            let mut row0 = [0u8; W];
            for (d, &g) in row0.iter_mut().zip(&self.grid[..cell]) {
                *d = g - b'a';
            }
            k.count_sym(r, c, row, &mut colv, &mut nodes, &mut row0)
        } else {
            k.count(r, c, row, &mut colv, &mut nodes, !0)
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
        let k = Kernel::<W> { rt: &s.row_trie, bi: &s.bigrams, ct: &s.col_trie, last_r: s.rows - 1 };
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
    rt: &'t Trie,
    /// `ParSearch::bigrams`
    bi: &'t [Bigram],
    ct: &'t CTrie,
    last_r: usize,
}

impl<const W: usize> Kernel<'_, W> {
    /// Last row, cell `c`: completions of `row` with letter `c'` in `masks[c']` for every c' >= c.
    fn count_last(&self, c: usize, row: Node, masks: &[u32; W], nodes: &mut u64) -> u64 {
        *nodes += 1;
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
                total += (self.rt.step(row, l).mask & last).count_ones() as u64;
            }
            return total;
        }
        let m = |k: usize| if k < W { masks[k] } else { !0 };
        let mut good = self.good(row, allowed, c, m(c + 1), m(c + 2), m(c + 3));
        while good != 0 {
            let l = good.trailing_zeros();
            good &= good - 1;
            total += self.count_last(c + 1, self.rt.step(row, l), masks, nodes);
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
            colv[c] = self.ct.step(col, l);
            let (nr, nc, nrow) = if c == (W - 1) { (r + 1, 0, self.rt.node(0)) } else { (r, c + 1, self.rt.step(row, l)) };
            total += if tie_cell && l > w {
                2 * self.count(nr, nc, nrow, colv, nodes, !0)
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
        let base = if last_row { !0 } else if c == 0 { self.rt.node(0).mask } else { self.uni(c - 1, f) };
        let cell = r * W + c;
        let mut keep_going = true;
        while allowed != 0 {
            let l = allowed.trailing_zeros();
            allowed &= allowed - 1;
            st.grid[cell] = b'a' + l as u8;
            let ok = if last_row && c == W - 1 {
                on(&st.grid)
            } else {
                let cn = self.ct.step(col, l);
                let fc = if last_row { !0 } else { base & cn.mask };
                if fc == 0 {
                    continue; // next row can't fit the columns
                }
                st.colv[c] = cn;
                if c == W - 1 {
                    self.search(r + 1, 0, self.rt.node(0), st, on, stop, !0)
                } else {
                    self.search(r, c + 1, self.rt.step(row, l), st, on, stop, fc)
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
    fn count(&self, r: usize, c: usize, row: Node, colv: &mut [CNode; W], nodes: &mut u64, f: u32) -> u64 {
        if r == self.last_r {
            // Last row: columns only need their masks (their children are never used).
            let mut masks = [0u32; W];
            for (m, n) in masks.iter_mut().zip(colv.iter()) {
                *m = n.mask;
            }
            return self.count_last(c, row, &masks, nodes);
        }
        *nodes += 1;
        let col = colv[c];
        let mut allowed = row.mask & col.mask;
        // letters row r + 1 could have at position c, before this cell's column constraint
        let base = if c == 0 { self.rt.node(0).mask } else { self.uni(c - 1, f) };
        if c == (W - 1) {
            // the letter completes this row: next cell is (r + 1, 0) with a fresh row
            let root = self.rt.node(0);
            let mut total = 0;
            while allowed != 0 {
                let l = allowed.trailing_zeros();
                allowed &= allowed - 1;
                let cn = self.ct.step(col, l);
                if base & cn.mask == 0 {
                    continue; // next row can't fit the columns
                }
                colv[c] = cn;
                total += self.count(r + 1, 0, root, colv, nodes, !0);
            }
            colv[c] = col;
            return total;
        }
        let mut total = 0;
        let m = |k: usize| if k < W { colv[k].mask } else { !0 };
        let mut good = self.good_fw(row, col, allowed, c, m(c + 1), m(c + 2), m(c + 3), base);
        while good != 0 {
            let l = good.trailing_zeros();
            good &= good - 1;
            let cn = self.ct.step(col, l);
            let fc = base & cn.mask;
            colv[c] = cn;
            total += self.count(r, c + 1, self.rt.step(row, l), colv, nodes, fc);
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
            let n = self.rt.step(row, l);
            let ok = (n.mask & m1 != 0) & (e2 | (n.next & m2 != 0)) & (e3 | (n.next2 & m3 != 0));
            good |= (ok as u32) << l;
        }
        good
    }

    /// `good` plus the next-row forward check (`base & column child mask != 0`), all branch-free.
    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    fn good_fw(&self, row: Node, col: CNode, allowed: u32, c: usize, m1: u32, m2: u32, m3: u32, base: u32) -> u32 {
        let (e2, e3) = (c + 2 >= W, c + 3 >= W);
        let mut a = allowed;
        let mut good = 0;
        while a != 0 {
            let l = a.trailing_zeros();
            a &= a - 1;
            let n = self.rt.step(row, l);
            let cn = self.ct.step(col, l);
            let ok = (n.mask & m1 != 0) & (e2 | (n.next & m2 != 0)) & (e3 | (n.next2 & m3 != 0)) & (base & cn.mask != 0);
            good |= (ok as u32) << l;
        }
        good
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
        ParSearch {
            rows,
            cols,
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
        // with symmetry, job prefixes must stay in row 0 (they are expanded without the tie rule)
        let max_depth = if self.sym { self.cols } else { usize::MAX };
        let (depth, jobs) = self.jobs(COUNT_JOBS_PER_THREAD, max_depth);
        let total: u64 = self.run(depth, &jobs, |w, _, _| Some(w.count_job(depth))).iter().map(|(_, c)| c).sum();
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
