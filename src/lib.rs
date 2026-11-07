pub mod dict;
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
