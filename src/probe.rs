//! Optional per-cell node counters (`cargo build --features probe`), for the search heatmap
//! (`scripts/heatmap.py`). Without the feature, `probe::hit` is an empty inline function.
//! Counters are thread-local (no contention) and merged into a global table when a worker ends.

/// Grid cells tracked: rows and columns up to 32. Row `LAST` collects the last-row kernel.
pub const N: usize = 32;

#[cfg(feature = "probe")]
mod imp {
    use super::N;
    use std::cell::Cell;
    use std::sync::Mutex;

    thread_local! {
        static LOCAL: [Cell<u64>; N * N] = const { [const { Cell::new(0) }; N * N] };
    }
    static GLOBAL: Mutex<[u64; N * N]> = Mutex::new([0; N * N]);

    #[inline(always)]
    pub fn hit(r: usize, c: usize) {
        LOCAL.with(|l| l[r * N + c].set(l[r * N + c].get() + 1));
    }

    /// Move this thread's counters into the global table.
    pub fn flush() {
        let mut g = GLOBAL.lock().unwrap();
        LOCAL.with(|l| {
            for (gi, li) in g.iter_mut().zip(l.iter()) {
                *gi += li.take();
            }
        });
    }

    /// Global table (after flushing the calling thread), row-major `N × N`.
    pub fn take() -> Vec<u64> {
        flush();
        GLOBAL.lock().unwrap().to_vec()
    }
}

#[cfg(not(feature = "probe"))]
mod imp {
    #[inline(always)]
    pub fn hit(_r: usize, _c: usize) {}
    #[inline(always)]
    pub fn flush() {}
    pub fn take() -> Vec<u64> {
        Vec::new()
    }
}

pub use imp::{flush, hit, take};

/// True when built with `--features probe`.
pub const ENABLED: bool = cfg!(feature = "probe");
