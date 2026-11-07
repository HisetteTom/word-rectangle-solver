//! Reference implementation. NEVER OPTIMIZE THIS FILE.
//!
//! Picks whole row words one at a time, and after each row checks that every
//! column read so far is a prefix of some column word (via a `HashSet` of prefixes).
//! Slow on purpose: it is the ground truth for correctness and the speed baseline.

use crate::{Grid, Stats};
use std::collections::HashSet;

pub struct Naive<'a> {
    rows: usize,
    cols: usize,
    row_words: &'a [String],
    col_prefixes: HashSet<String>,
    grid: Vec<&'a str>,
    pub stats: Stats,
}

impl<'a> Naive<'a> {
    /// `row_words`: words of length `cols`. `col_words`: words of length `rows`.
    pub fn new(rows: usize, cols: usize, row_words: &'a [String], col_words: &[String]) -> Self {
        let mut col_prefixes = HashSet::new();
        for w in col_words {
            for k in 1..=w.len() {
                col_prefixes.insert(w[..k].to_string());
            }
        }
        Naive { rows, cols, row_words, col_prefixes, grid: Vec::new(), stats: Stats::default() }
    }

    fn columns_ok(&self) -> bool {
        (0..self.cols).all(|c| {
            let col: String = self.grid.iter().map(|w| w.as_bytes()[c] as char).collect();
            self.col_prefixes.contains(&col)
        })
    }

    /// Calls `on_solution` for every solution; stops early if it returns `false`.
    fn search(&mut self, on_solution: &mut dyn FnMut(&[&str]) -> bool) -> bool {
        self.stats.nodes += 1;
        if self.grid.len() == self.rows {
            self.stats.solutions += 1;
            return on_solution(&self.grid);
        }
        for w in self.row_words {
            self.grid.push(w);
            let keep_going = !self.columns_ok() || self.search(on_solution);
            self.grid.pop();
            if !keep_going {
                return false;
            }
        }
        true
    }

    /// First solution found, if any.
    pub fn find(&mut self) -> Option<Grid> {
        let mut found = None;
        self.search(&mut |g| {
            found = Some(g.iter().map(|s| s.to_string()).collect());
            false
        });
        found
    }

    /// Number of solutions (exhaustive).
    pub fn count(&mut self) -> u64 {
        self.search(&mut |_| true);
        self.stats.solutions
    }

    /// Every solution (only for small cases / tests).
    pub fn all(&mut self) -> Vec<Grid> {
        let mut out = Vec::new();
        self.search(&mut |g| {
            out.push(g.iter().map(|s| s.to_string()).collect());
            true
        });
        out
    }
}
