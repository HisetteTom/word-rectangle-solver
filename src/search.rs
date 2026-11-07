//! Level 1 solver: cell-by-cell backtracking over two tries (README §7).
//!
//! For the next cell (row-major), the legal letters are
//! `row_trie.mask(row_state[r]) & col_trie.mask(col_state[c])`.
//! "nodes" = calls to `search` = cells tried, NOT comparable to naive's nodes.
//! In count mode the last cell is counted with a popcount, not visited (fewer nodes).

use crate::trie::Trie;
use crate::{Grid, Solver, Stats};

pub struct TrieSearch {
    rows: usize,
    cols: usize,
    row_trie: Trie, // words of length `cols`
    col_trie: Trie, // words of length `rows`
    row_state: Vec<u32>,
    col_state: Vec<u32>,
    grid: Vec<u8>,
    stats: Stats,
}

impl TrieSearch {
    /// `row_words`: words of length `cols`. `col_words`: words of length `rows`.
    pub fn new(rows: usize, cols: usize, row_words: &[String], col_words: &[String]) -> Self {
        TrieSearch {
            rows,
            cols,
            row_trie: Trie::new(row_words),
            col_trie: Trie::new(col_words),
            row_state: vec![0; rows],
            col_state: vec![0; cols],
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
    fn search<F: FnMut(&Self) -> bool>(&mut self, cell: usize, on_solution: &mut F) -> bool {
        self.stats.nodes += 1;
        if cell == self.rows * self.cols {
            self.stats.solutions += 1;
            return on_solution(self);
        }
        let (r, c) = (cell / self.cols, cell % self.cols);
        let row_node = self.row_state[r];
        let col_node = self.col_state[c];
        let mut allowed = self.row_trie.mask(row_node) & self.col_trie.mask(col_node);
        let mut keep_going = true;
        while allowed != 0 {
            let letter = allowed.trailing_zeros() as usize;
            allowed &= allowed - 1; // clear lowest set bit
            self.row_state[r] = self.row_trie.child(row_node, letter);
            self.col_state[c] = self.col_trie.child(col_node, letter);
            self.grid[cell] = b'a' + letter as u8;
            if !self.search(cell + 1, on_solution) {
                keep_going = false;
                break;
            }
        }
        self.row_state[r] = row_node;
        self.col_state[c] = col_node;
        keep_going
    }

    /// Counting-only search: no grid writes, no callback, and the last cell is not
    /// recursed into — its number of completions is just `popcount(allowed)`.
    fn count_rec(&mut self, cell: usize) -> u64 {
        self.stats.nodes += 1;
        let (r, c) = (cell / self.cols, cell % self.cols);
        let row_node = self.row_state[r];
        let col_node = self.col_state[c];
        let mut allowed = self.row_trie.mask(row_node) & self.col_trie.mask(col_node);
        if cell + 1 == self.rows * self.cols {
            return allowed.count_ones() as u64;
        }
        let mut total = 0;
        while allowed != 0 {
            let letter = allowed.trailing_zeros() as usize;
            allowed &= allowed - 1;
            self.row_state[r] = self.row_trie.child(row_node, letter);
            self.col_state[c] = self.col_trie.child(col_node, letter);
            total += self.count_rec(cell + 1);
        }
        self.row_state[r] = row_node;
        self.col_state[c] = col_node;
        total
    }
}

impl Solver for TrieSearch {
    fn find(&mut self) -> Option<Grid> {
        let mut found = None;
        self.search(0, &mut |s: &Self| {
            found = Some(s.to_grid());
            false
        });
        found
    }

    fn count(&mut self) -> u64 {
        self.stats.solutions = self.count_rec(0);
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
