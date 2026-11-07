//! Independent, dead-simple checker. Shares no code with any solver.

use std::collections::HashSet;

/// Returns `Err(reason)` if the grid is not a valid word rectangle for `dict`.
pub fn validate(grid: &[String], dict: &HashSet<String>) -> Result<(), String> {
    let rows = grid.len();
    if rows == 0 {
        return Err("empty grid".into());
    }
    let cols = grid[0].len();
    for (i, row) in grid.iter().enumerate() {
        if row.len() != cols {
            return Err(format!("row {i} has length {} (expected {cols})", row.len()));
        }
        if !dict.contains(row) {
            return Err(format!("row {i} '{row}' is not a word"));
        }
    }
    for c in 0..cols {
        let col: String = grid.iter().map(|r| r.as_bytes()[c] as char).collect();
        if !dict.contains(&col) {
            return Err(format!("column {c} '{col}' is not a word"));
        }
    }
    Ok(())
}
