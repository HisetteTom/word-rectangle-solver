pub mod dict;
pub mod naive;
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
