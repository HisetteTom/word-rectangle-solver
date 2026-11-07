//! Level 3 solver: "staircase" most-constrained-cell-first (MRV) + frontier forward checking.
//!
//! A cell (r, c) can be filled once its row prefix (r, 0..c) and column prefix (0..r, c)
//! are filled, because both tries are walked from the start of the word. So the fillable
//! cells form the corners of a staircase: row r's next cell `c = row_fill[r]` is fillable
//! iff `col_fill[c] == r`. At every node we look at all frontier cells:
//!   - if any has `row_mask & col_mask == 0`, the branch is dead (forward check);
//!   - otherwise we branch on the one with the fewest legal letters.
//!
//! Every solution is still reached exactly once (we branch on all letters of one cell).

use crate::trie::Trie;
use crate::{Grid, Solver, Stats};

pub struct Mrv {
    rows: usize,
    cols: usize,
    row_trie: Trie, // words of length `cols`
    col_trie: Trie, // words of length `rows`
    row_state: Vec<u32>,
    col_state: Vec<u32>,
    row_fill: Vec<usize>,
    col_fill: Vec<usize>,
    grid: Vec<u8>,
    stats: Stats,
}

impl Mrv {
    /// `row_words`: words of length `cols`. `col_words`: words of length `rows`.
    pub fn new(rows: usize, cols: usize, row_words: &[String], col_words: &[String]) -> Self {
        Mrv {
            rows,
            cols,
            row_trie: Trie::new(row_words),
            col_trie: Trie::new(col_words),
            row_state: vec![0; rows],
            col_state: vec![0; cols],
            row_fill: vec![0; rows],
            col_fill: vec![0; cols],
            grid: vec![0; rows * cols],
            stats: Stats::default(),
        }
    }

    fn to_grid(&self) -> Grid {
        self.grid
            .chunks(self.cols)
            .map(|r| String::from_utf8(r.to_vec()).unwrap())
            .collect()
    }

    /// Calls `on_solution` for every solution; stops early if it returns `false`.
    fn search<F: FnMut(&Self) -> bool>(&mut self, filled: usize, on_solution: &mut F) -> bool {
        self.stats.nodes += 1;
        if filled == self.rows * self.cols {
            self.stats.solutions += 1;
            return on_solution(self);
        }
        // pick the most constrained frontier cell; any empty one kills the branch
        let (mut br, mut bc, mut bm, mut bk) = (0, 0, 0u32, u32::MAX);
        for r in 0..self.rows {
            let c = self.row_fill[r];
            if c < self.cols && self.col_fill[c] == r {
                let m = self.row_trie.mask(self.row_state[r]) & self.col_trie.mask(self.col_state[c]);
                let k = m.count_ones();
                if k == 0 {
                    return true; // dead end, keep searching elsewhere
                }
                if k < bk {
                    (br, bc, bm, bk) = (r, c, m, k);
                }
            }
        }
        let (r, c) = (br, bc);
        let row_node = self.row_state[r];
        let col_node = self.col_state[c];
        self.row_fill[r] += 1;
        self.col_fill[c] += 1;
        let mut allowed = bm;
        let mut keep_going = true;
        while allowed != 0 {
            let letter = allowed.trailing_zeros() as usize;
            allowed &= allowed - 1;
            self.row_state[r] = self.row_trie.child(row_node, letter);
            self.col_state[c] = self.col_trie.child(col_node, letter);
            self.grid[r * self.cols + c] = b'a' + letter as u8;
            if !self.search(filled + 1, on_solution) {
                keep_going = false;
                break;
            }
        }
        self.row_fill[r] -= 1;
        self.col_fill[c] -= 1;
        self.row_state[r] = row_node;
        self.col_state[c] = col_node;
        keep_going
    }
}

impl Solver for Mrv {
    fn find(&mut self) -> Option<Grid> {
        let mut found = None;
        self.search(0, &mut |s: &Self| {
            found = Some(s.to_grid());
            false
        });
        found
    }

    fn count(&mut self) -> u64 {
        self.search(0, &mut |_: &Self| true);
        self.stats.solutions
    }

    fn all(&mut self) -> Vec<Grid> {
        let mut out = Vec::new();
        self.search(0, &mut |s: &Self| {
            out.push(s.to_grid());
            true
        });
        out
    }

    fn stats(&self) -> Stats {
        self.stats
    }
}
