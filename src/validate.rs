//! Independent, dead-simple checker. Shares no code with any solver.

/// Returns `Err(reason)` if the grid is not a valid word rectangle, `is_word` being the dictionary
/// (e.g. `|w| set.contains(w)`).
pub fn validate(grid: &[String], is_word: impl Fn(&str) -> bool) -> Result<(), String> {
    let rows = grid.len();
    if rows == 0 {
        return Err("empty grid".into());
    }
    let cols = grid[0].len();
    for (i, row) in grid.iter().enumerate() {
        if row.len() != cols {
            return Err(format!("row {i} has length {} (expected {cols})", row.len()));
        }
        if !is_word(row) {
            return Err(format!("row {i} '{row}' is not a word"));
        }
    }
    for c in 0..cols {
        let col: String = grid.iter().map(|r| r.as_bytes()[c] as char).collect();
        if !is_word(&col) {
            return Err(format!("column {c} '{col}' is not a word"));
        }
    }
    Ok(())
}
