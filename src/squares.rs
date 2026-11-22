//! Symmetric word squares: n×n grids where row i equals column i (the classic puzzle), so one word
//! list serves both. A special case of square rectangles, counted on its own for the atlas.
//!
//! Search: rows top to bottom. By symmetry, the first r letters of row r are already placed
//! (cell (r, i) = cell (i, r) for i < r), so row r walks the trie along that forced prefix and
//! branches only on positions r..n, where each letter must also continue column c's prefix (column
//! c will become row c). Parallel over the first row (std threads, atomic job counter).

use crate::trie::Trie;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};

pub struct Squares {
    n: usize,
    trie: Trie,
    first_rows: Vec<Vec<u8>>,
}

impl Squares {
    /// `words` must all have length `n` (lowercase a–z).
    pub fn new(n: usize, words: &[String]) -> Squares {
        let first_rows = words.iter().map(|w| w.bytes().map(|b| b - b'a').collect()).collect();
        Squares { n, trie: Trie::new(words), first_rows }
    }

    /// Number of symmetric word squares (each counted once), and the search nodes visited.
    pub fn count(&self, threads: usize) -> (u64, u64) {
        let threads = if threads == 0 { std::thread::available_parallelism().map_or(1, |n| n.get()) } else { threads };
        let (next, total, nodes) = (AtomicUsize::new(0), AtomicU64::new(0), AtomicU64::new(0));
        std::thread::scope(|sc| {
            for _ in 0..threads {
                sc.spawn(|| {
                    let mut grid = vec![0u8; self.n * self.n];
                    let mut col = vec![0u32; self.n];
                    let (mut t, mut nd) = (0, 0);
                    loop {
                        let j = next.fetch_add(1, Relaxed);
                        let Some(row0) = self.first_rows.get(j) else { break };
                        grid[..self.n].copy_from_slice(row0);
                        // column c starts with row 0's letter c
                        let mut ok = true;
                        for (c, &l) in row0.iter().enumerate() {
                            let root = 0;
                            if self.trie.mask(root) >> l & 1 == 0 {
                                ok = false;
                                break;
                            }
                            col[c] = self.trie.child(root, l as usize);
                        }
                        if ok {
                            t += self.row(1, &mut grid, &mut col, &mut nd);
                        }
                    }
                    total.fetch_add(t, Relaxed);
                    nodes.fetch_add(nd, Relaxed);
                });
            }
        });
        (total.into_inner(), nodes.into_inner())
    }

    /// Count completions from row `r` (rows < r placed; `col[c]` = trie node of column c's prefix).
    fn row(&self, r: usize, grid: &mut [u8], col: &mut [u32], nodes: &mut u64) -> u64 {
        if r == self.n {
            return 1;
        }
        // forced prefix: (r, i) = (i, r) for i < r. Column r's state already spells it.
        let node = col[r];
        self.cell(r, r, node, grid, col, nodes)
    }

    /// Place cell (r, c), c >= r; `node` = row r's trie state after positions 0..c.
    fn cell(&self, r: usize, c: usize, node: u32, grid: &mut [u8], col: &mut [u32], nodes: &mut u64) -> u64 {
        *nodes += 1;
        if c == self.n {
            return self.row(r + 1, grid, col, nodes);
        }
        // the diagonal cell (r, r) only continues row r (= column r); others must also continue
        // column c, whose state after rows 0..r is col[c]
        let mut allowed = self.trie.mask(node);
        if c > r {
            allowed &= self.trie.mask(col[c]);
        }
        let mut total = 0;
        while allowed != 0 {
            let l = allowed.trailing_zeros() as usize;
            allowed &= allowed - 1;
            grid[r * self.n + c] = l as u8;
            let saved = col[c];
            if c > r {
                col[c] = self.trie.child(saved, l);
            }
            total += self.cell(r, c + 1, self.trie.child(node, l), grid, col, nodes);
            col[c] = saved;
        }
        total
    }
}
