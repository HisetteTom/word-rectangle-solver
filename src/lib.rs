pub mod dict;
pub mod mrv;
pub mod naive;
pub mod search;
pub mod trie;
pub mod validate;

/// A solved grid: `rows` words of `cols` letters each.
pub type Grid = Vec<String>;

/// Search statistics, shared by every solver so numbers are comparable.
#[derive(Debug, Default, Clone, Copy)]
pub struct Stats {
    /// Number of search nodes visited (calls to the recursive step).
    pub nodes: u64,
    /// Number of complete solutions found.
    pub solutions: u64,
}

/// Every solver, by CLI name. The first one is the default.
pub const SOLVERS: &[&str] = &["mrv", "trie", "naive"];

/// Build a solver by name. `row_words` have length `cols`, `col_words` length `rows`.
pub fn make_solver<'a>(
    name: &str,
    rows: usize,
    cols: usize,
    row_words: &'a [String],
    col_words: &'a [String],
) -> Option<Box<dyn Solver + 'a>> {
    Some(match name {
        "naive" => Box::new(naive::Naive::new(rows, cols, row_words, col_words)),
        "trie" => Box::new(search::TrieSearch::new(rows, cols, row_words, col_words)),
        "mrv" => Box::new(mrv::Mrv::new(rows, cols, row_words, col_words)),
        _ => return None,
    })
}

/// Common interface so the CLI and tests can swap solvers.
pub trait Solver {
    /// First solution found, if any.
    fn find(&mut self) -> Option<Grid>;
    /// Number of solutions (exhaustive).
    fn count(&mut self) -> u64;
    /// Every solution (only for small cases / tests).
    fn all(&mut self) -> Vec<Grid>;
    fn stats(&self) -> Stats;
}
