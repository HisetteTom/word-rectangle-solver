//! Level 6: parallel version of the row-major trie search (`search.rs`), std threads only.
//! The hot loops (`Kernel`) carry trie nodes by value and track (row, col) directly.
//!
//! 0. `find` first tries a short sequential search (`FIND_SEQ_BUDGET` nodes): small cases stay cheap.
//! 1. Split: expand the first cells breadth-first (lexicographic order) until there are
//!    enough jobs (`*_JOBS_PER_THREAD` × threads). A job = the letters of that prefix.
//! 2. Workers pull jobs from an atomic counter (dynamic load balancing: subtrees are
//!    wildly uneven, and P-cores / E-cores run at different speeds).
//! 3. `find` is deterministic: `best` = lowest job index that found a solution; workers
//!    skip / abort any job after it, so the answer is the lexicographically first grid,
//!    exactly what the sequential solver returns.

use crate::trie::{Node, Trie};
use crate::{Grid, Solver, Stats};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};

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
    col_trie: Trie,
    threads: usize,
    seq_budget: u64,
    stats: Stats,
}

/// Per-thread search state.
struct Worker<'a> {
    s: &'a ParSearch,
    row_state: Vec<u32>,
    col_state: Vec<u32>,
    grid: Vec<u8>,
    nodes: u64,
    /// `search` gives up once `nodes` exceeds this.
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
        let (cols, s) = (self.s.cols, self.s);
        let (r, c) = (cell / cols, cell % cols);
        let mut colv: Vec<Node> = self.col_state.iter().map(|&n| s.col_trie.node(n)).collect();
        let k = Kernel { rt: &s.row_trie, ct: &s.col_trie, last_r: s.rows - 1, last_c: cols - 1 };
        let mut nodes = 0;
        let total = k.count(r, c, s.row_trie.node(self.row_state[r]), &mut colv, &mut nodes);
        self.nodes += nodes;
        total
    }

    /// Calls `on` on every solution of the loaded job prefix (`cell` = first empty cell);
    /// returns false if stopped (`on` returned false, `stop()` became true, or the node limit hit).
    fn search<F: FnMut(&[u8]) -> bool, S: Fn() -> bool>(&mut self, cell: usize, on: &mut F, stop: &S) -> bool {
        let s = self.s;
        if cell == s.rows * s.cols {
            return on(&self.grid);
        }
        let (r, c) = (cell / s.cols, cell % s.cols);
        let k = Kernel { rt: &s.row_trie, ct: &s.col_trie, last_r: s.rows - 1, last_c: s.cols - 1 };
        let mut st = SearchState {
            colv: self.col_state.iter().map(|&n| s.col_trie.node(n)).collect(),
            grid: std::mem::take(&mut self.grid),
            cols: s.cols,
            nodes: self.nodes,
            limit: self.limit,
        };
        let done = k.search(r, c, s.row_trie.node(self.row_state[r]), &mut st, on, stop);
        self.grid = st.grid;
        self.nodes = st.nodes;
        done
    }
}

/// Hot counting recursion. Carries trie nodes *by value*: `allowed` needs no memory
/// load, only the two child steps do. (r, c) are tracked directly (no `/` or `%`), and
/// the row state is a single node (finished rows are never needed again).
struct Kernel<'t> {
    rt: &'t Trie,
    ct: &'t Trie,
    last_r: usize,
    last_c: usize,
}

/// Mutable state of `Kernel::search`.
struct SearchState {
    colv: Vec<Node>,
    grid: Vec<u8>,
    cols: usize,
    nodes: u64,
    limit: u64,
}

impl Kernel<'_> {
    /// Same order as `count`, writes letters to the grid; false = stop.
    fn search<F: FnMut(&[u8]) -> bool, S: Fn() -> bool>(&self, r: usize, c: usize, row: Node, st: &mut SearchState, on: &mut F, stop: &S) -> bool {
        st.nodes += 1;
        if st.nodes > st.limit || stop() {
            return false;
        }
        let col = st.colv[c];
        let mut allowed = row.mask & col.mask;
        let cell = r * st.cols + c;
        let mut keep_going = true;
        while allowed != 0 {
            let l = allowed.trailing_zeros();
            allowed &= allowed - 1;
            st.grid[cell] = b'a' + l as u8;
            let ok = if c == self.last_c {
                if r == self.last_r {
                    on(&st.grid)
                } else {
                    st.colv[c] = self.ct.step(col, l);
                    self.search(r + 1, 0, self.rt.node(0), st, on, stop)
                }
            } else {
                st.colv[c] = self.ct.step(col, l);
                self.search(r, c + 1, self.rt.step(row, l), st, on, stop)
            };
            if !ok {
                keep_going = false;
                break;
            }
        }
        st.colv[c] = col;
        keep_going
    }

    fn count(&self, r: usize, c: usize, row: Node, colv: &mut [Node], nodes: &mut u64) -> u64 {
        *nodes += 1;
        let col = colv[c];
        let mut allowed = row.mask & col.mask;
        if c == self.last_c {
            if r == self.last_r {
                return allowed.count_ones() as u64; // each letter completes the grid
            }
            // the letter completes this row: next cell is (r + 1, 0) with a fresh row
            let root = self.rt.node(0);
            let mut total = 0;
            while allowed != 0 {
                let l = allowed.trailing_zeros();
                allowed &= allowed - 1;
                colv[c] = self.ct.step(col, l);
                total += self.count(r + 1, 0, root, colv, nodes);
            }
            colv[c] = col;
            return total;
        }
        let mut total = 0;
        while allowed != 0 {
            let l = allowed.trailing_zeros();
            allowed &= allowed - 1;
            colv[c] = self.ct.step(col, l);
            total += self.count(r, c + 1, self.rt.step(row, l), colv, nodes);
        }
        colv[c] = col;
        total
    }
}

impl ParSearch {
    /// `threads == 0` = all available cores.
    pub fn new(rows: usize, cols: usize, row_words: &[String], col_words: &[String], threads: usize) -> Self {
        let threads = if threads == 0 {
            std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
        } else {
            threads
        };
        ParSearch {
            rows,
            cols,
            row_trie: Trie::new(row_words),
            col_trie: Trie::new(col_words),
            threads,
            seq_budget: FIND_SEQ_BUDGET,
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
    fn jobs(&mut self, per_thread: usize) -> (usize, Vec<u8>) {
        let n = self.rows * self.cols;
        let target = per_thread * self.threads;
        let mut w = Worker::new(self);
        let mut depth = 0;
        let mut jobs: Vec<u8> = Vec::new(); // flattened prefixes of length `depth`
        let mut count = 1;
        let mut expanded = 0;
        while count < target && depth + 1 < n {
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
        let (depth, jobs) = self.jobs(FIND_JOBS_PER_THREAD);
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
        let (depth, jobs) = self.jobs(COUNT_JOBS_PER_THREAD);
        let total: u64 = self.run(depth, &jobs, |w, _, _| Some(w.count_job(depth))).iter().map(|(_, c)| c).sum();
        self.stats.solutions = total;
        total
    }

    fn all(&mut self) -> Vec<Grid> {
        let (depth, jobs) = self.jobs(COUNT_JOBS_PER_THREAD);
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
