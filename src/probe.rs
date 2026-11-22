//! Optional per-cell node counters (`cargo build --features probe`), for the search heatmap
//! (`scripts/heatmap.py`). Without the feature, `probe::hit` is an empty inline function.
//! Counters are thread-local (no contention) and merged into a global table when a worker ends.
//! Also a timeline of `find`'s parallel work (`event`, for `scripts/timeline.py`).

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

    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
    use std::time::Instant;
    static EPOCH: Mutex<Option<Instant>> = Mutex::new(None);
    static EVENTS: Mutex<Vec<super::Event>> = Mutex::new(Vec::new());
    static LANES: AtomicUsize = AtomicUsize::new(0);
    thread_local! {
        static LANE: usize = LANES.fetch_add(1, Relaxed);
    }

    /// Start a new timeline (time 0 = now).
    pub fn epoch() {
        *EPOCH.lock().unwrap() = Some(Instant::now());
        EVENTS.lock().unwrap().clear();
    }

    /// Record work of `kind` on this thread, from `start` to now.
    pub fn event(kind: u8, start: Instant, rank: u64) {
        let Some(e) = *EPOCH.lock().unwrap() else { return };
        let ev = super::Event { lane: LANE.with(|l| *l), kind, start_us: start.duration_since(e).as_secs_f64() * 1e6, end_us: e.elapsed().as_secs_f64() * 1e6, rank };
        EVENTS.lock().unwrap().push(ev);
    }

    pub fn events() -> Vec<super::Event> {
        EVENTS.lock().unwrap().clone()
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
    #[inline(always)]
    pub fn epoch() {}
    #[inline(always)]
    pub fn event(_kind: u8, _start: std::time::Instant, _rank: u64) {}
    pub fn events() -> Vec<super::Event> {
        Vec::new()
    }
}

pub use imp::{epoch, event, events, flush, hit, take};

/// One piece of `find`'s parallel work (timeline): thread `lane`, `kind` (see `KINDS`), times in µs
/// since `epoch()`, `rank` = the job's rank (top job index = `rank >> 40`).
#[derive(Clone, Copy, Debug)]
pub struct Event {
    pub lane: usize,
    pub kind: u8,
    pub start_us: f64,
    pub end_us: f64,
    pub rank: u64,
}

/// Names of `Event::kind`.
pub const KINDS: [&str; 6] = ["test-empty", "test-positive", "aborted-split", "search-found", "part", "cut"];

/// True when built with `--features probe`.
pub const ENABLED: bool = cfg!(feature = "probe");
