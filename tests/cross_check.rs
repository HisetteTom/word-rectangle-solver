//! Every optimized solver must agree exactly with the naive reference.
//! New solver? It is picked up automatically from `wordrect::SOLVERS`.

use std::collections::HashSet;
use wordrect::dict::Dict;
use wordrect::validate::validate;
use wordrect::par::ParSearch;
use wordrect::{Grid, SOLVERS, Solver, make_solver};

fn optimized() -> impl Iterator<Item = &'static str> {
    SOLVERS.iter().copied().filter(|&s| s != "naive")
}

fn all(name: &str, d: &Dict, r: usize, c: usize) -> Vec<Grid> {
    let mut v = make_solver(name, r, c, d.words(c), d.words(r)).unwrap().all();
    v.sort();
    v
}

fn count(name: &str, d: &Dict, r: usize, c: usize) -> u64 {
    make_solver(name, r, c, d.words(c), d.words(r)).unwrap().count()
}

fn find(name: &str, d: &Dict, r: usize, c: usize) -> Option<Grid> {
    make_solver(name, r, c, d.words(c), d.words(r)).unwrap().find()
}

fn set(d: &Dict) -> HashSet<String> {
    d.by_len.iter().flatten().cloned().collect()
}

#[test]
fn match_naive_on_pseudo_random_dicts() {
    let mut seed: u64 = 987654321;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as usize
    };
    for _ in 0..60 {
        let words: Vec<String> = (0..20)
            .map(|_| {
                let len = 2 + next() % 3;
                (0..len).map(|_| (b'a' + (next() % 3) as u8) as char).collect()
            })
            .collect();
        let d = Dict::from_words(words.iter().map(String::as_str));
        for (r, c) in [(2, 2), (2, 3), (3, 2), (3, 3), (2, 4), (4, 3)] {
            let want = all("naive", &d, r, c);
            for s in optimized() {
                assert_eq!(all(s, &d, r, c), want, "solver={s} dict={words:?} size={r}x{c}");
                assert_eq!(count(s, &d, r, c), want.len() as u64, "solver={s} count");
                assert_eq!(find(s, &d, r, c).is_some(), !want.is_empty(), "solver={s} find");
            }
        }
    }
}

#[test]
fn match_naive_on_enable_small_sizes() {
    let d = Dict::load("dicts/enable.txt".as_ref()).unwrap();
    let dict = set(&d);
    for (r, c) in [(2, 2), (2, 3), (3, 2)] {
        let want = all("naive", &d, r, c);
        assert!(!want.is_empty());
        for s in optimized() {
            let got = all(s, &d, r, c);
            assert_eq!(got, want, "solver={s} size={r}x{c}");
            for g in &got {
                validate(g, |w| dict.contains(w)).unwrap();
            }
        }
    }
}

#[test]
fn enable_3x3_count_and_finds_validate() {
    let d = Dict::load("dicts/enable.txt".as_ref()).unwrap();
    let dict = set(&d);
    for s in optimized() {
        assert_eq!(count(s, &d, 3, 3), 1_909_732, "solver={s}");
        for (r, c) in [(3, 4), (4, 4), (5, 5)] {
            let g = find(s, &d, r, c).unwrap_or_else(|| panic!("solver={s} found nothing for {r}x{c}"));
            validate(&g, |w| dict.contains(w)).unwrap();
        }
    }
}

#[test]
fn row_major_solvers_find_same_first_solution_as_naive() {
    // Both explore in lexicographic order, so the first grid must be identical.
    let d = Dict::load("dicts/enable.txt".as_ref()).unwrap();
    for (r, c) in [(3, 3), (3, 4), (4, 4)] {
        let want = find("naive", &d, r, c);
        for s in optimized() {
            assert_eq!(find(s, &d, r, c), want, "solver={s} size={r}x{c}");
        }
    }
}

#[test]
fn empty_and_tiny() {
    for s in optimized() {
        let d = Dict::from_words(["cat", "ape", "ten"]);
        assert_eq!(all(s, &d, 3, 3), vec![vec!["cat".to_string(), "ape".into(), "ten".into()]], "{s}");
        let d = Dict::from_words(["cat", "dog"]);
        assert!(all(s, &d, 3, 3).is_empty(), "{s}");
        assert!(all(s, &d, 5, 5).is_empty(), "{s}: no words of that length");
        assert!(find(s, &d, 5, 5).is_none(), "{s}");
        let d = Dict::from_words(["a", "b"]);
        assert_eq!(count(s, &d, 1, 1), 2, "{s}: 1x1");
        assert_eq!(all(s, &d, 1, 1).len(), 2, "{s}: 1x1");
    }
}

#[test]
fn par_parallel_find_path_is_deterministic() {
    // seq budget 0 forces the multi-threaded path; must still return naive's first grid.
    let d = Dict::load("dicts/enable.txt".as_ref()).unwrap();
    for (r, c) in [(2, 2), (3, 3), (3, 4), (4, 4), (2, 6)] {
        let want = find("naive", &d, r, c);
        for threads in [1, 3, 0] {
            let got = ParSearch::new(r, c, d.words(c), d.words(r), threads).with_seq_budget(0).find();
            assert_eq!(got, want, "size={r}x{c} threads={threads}");
        }
    }
    let d = Dict::from_words(["cat", "dog"]);
    assert!(ParSearch::new(3, 3, d.words(3), d.words(3), 0).with_seq_budget(0).find().is_none());
}

#[test]
fn par_split_jobs_find_naive_first_grid() {
    // every job split into sub-jobs, recursively: the answer must still be naive's first grid
    let d = Dict::load("dicts/enable.txt".as_ref()).unwrap();
    for (r, c) in [(3, 3), (3, 4), (4, 4), (2, 6), (4, 5)] {
        let want = find("naive", &d, r, c);
        for threads in [1, 3, 0] {
            let got = ParSearch::new(r, c, d.words(c), d.words(r), threads).with_seq_budget(0).with_eager_split().find();
            assert_eq!(got, want, "size={r}x{c} threads={threads}");
        }
    }
    let mut seed: u64 = 7;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as usize
    };
    let (mut some, mut none) = (0, 0);
    for (r, c, n_words) in [(4, 5, 6), (5, 5, 5), (3, 7, 4), (5, 6, 6)] {
        for _ in 0..8 {
            let word = |len: usize, next: &mut dyn FnMut() -> usize| -> String { (0..len).map(|_| (b'a' + (next() % 2) as u8) as char).collect() };
            let mut words: Vec<String> = (0..n_words).map(|_| word(c, &mut next)).collect();
            words.extend((0..n_words).map(|_| word(r, &mut next)));
            let d = Dict::from_words(words.iter().map(String::as_str));
            let want = find("naive", &d, r, c);
            if want.is_some() { some += 1 } else { none += 1 }
            for threads in [1, 3, 0] {
                let got = ParSearch::new(r, c, d.words(c), d.words(r), threads).with_seq_budget(0).with_eager_split().find();
                assert_eq!(got, want, "size={r}x{c} threads={threads} dict={words:?}");
            }
        }
    }
    assert!(some > 0 && none > 0, "want both outcomes: some={some} none={none}");
}

#[test]
fn par_aborted_jobs_resume_and_find_naive_first_grid() {
    // every job aborted at the kernels' first check and split, skipping what it already explored
    let d = Dict::load("dicts/enable.txt".as_ref()).unwrap();
    for (r, c) in [(3, 4), (4, 4), (2, 6), (4, 5), (3, 6), (5, 5), (4, 6), (3, 8), (5, 6)] {
        let want = find("naive", &d, r, c);
        for threads in [1, 3, 0] {
            let got = ParSearch::new(r, c, d.words(c), d.words(r), threads).with_seq_budget(0).with_abort_split().find();
            assert_eq!(got, want, "size={r}x{c} threads={threads}");
        }
    }
    // mirror-tested sizes, with dense 2-letter dictionaries (enough nodes to hit the checks)
    let mut seed: u64 = 11;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as usize
    };
    let (mut some, mut none) = (0, 0);
    for (r, c, n_words) in [(6, 8, 60), (6, 8, 30), (5, 10, 80), (4, 6, 20), (5, 5, 12), (6, 8, 12), (6, 9, 9), (5, 10, 14)] {
        for _ in 0..4 {
            let word = |len: usize, next: &mut dyn FnMut() -> usize| -> String { (0..len).map(|_| (b'a' + (next() % 2) as u8) as char).collect() };
            let mut words: Vec<String> = (0..n_words).map(|_| word(c, &mut next)).collect();
            words.extend((0..n_words).map(|_| word(r, &mut next)));
            let d = Dict::from_words(words.iter().map(String::as_str));
            let want = find("naive", &d, r, c);
            if want.is_some() { some += 1 } else { none += 1 }
            for threads in [1, 3, 0] {
                let got = ParSearch::new(r, c, d.words(c), d.words(r), threads).with_seq_budget(0).with_sync_mirror().with_abort_split().find();
                assert_eq!(got, want, "size={r}x{c} threads={threads} dict={words:?}");
            }
        }
    }
    assert!(some > 0 && none > 0, "want both outcomes: some={some} none={none}");
}

/// Big squares take special count paths in `par` (mirror image + anti-diagonal symmetry from 6×6
/// up), which the small sizes above never reach. Dense random dictionaries over a 2-letter
/// alphabet give many solutions, including symmetric ones (the tie paths).
#[test]
fn match_naive_on_big_random_squares() {
    let mut seed: u64 = 1234567;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as usize
    };
    for n in [6usize, 7] {
        for _ in 0..12 {
            let words: Vec<String> = (0..14).map(|_| (0..n).map(|_| (b'a' + (next() % 2) as u8) as char).collect()).collect();
            let d = Dict::from_words(words.iter().map(String::as_str));
            let want = all("naive", &d, n, n);
            for s in optimized() {
                assert_eq!(count(s, &d, n, n), want.len() as u64, "solver={s} dict={words:?} size={n}x{n}");
            }
        }
    }
}

/// Big grids (area >= 48, rows of 8+ letters) test `find`'s parallel jobs on the mirror image, with
/// the job prefix forced at the end of the mirrored row 0. The answer must still be naive's
/// first grid (or none). Dense random dictionaries over 2 letters: some have solutions, some don't.
#[test]
fn par_mirror_job_test_matches_naive_first_grid() {
    let mut seed: u64 = 42;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as usize
    };
    let (mut some, mut none) = (0, 0);
    for (r, c, n_words) in [(6, 8, 12), (6, 8, 7), (5, 10, 14), (6, 9, 9), (8, 7, 10), (8, 7, 16)] {
        for _ in 0..6 {
            let word = |len: usize, next: &mut dyn FnMut() -> usize| -> String { (0..len).map(|_| (b'a' + (next() % 2) as u8) as char).collect() };
            let mut words: Vec<String> = (0..n_words).map(|_| word(c, &mut next)).collect();
            words.extend((0..n_words).map(|_| word(r, &mut next)));
            let d = Dict::from_words(words.iter().map(String::as_str));
            let want = find("naive", &d, r, c);
            if want.is_some() { some += 1 } else { none += 1 }
            for threads in [1, 3, 0] {
                for (sync, eager) in [(false, false), (true, false), (true, true)] {
                    let p = ParSearch::new(r, c, d.words(c), d.words(r), threads).with_seq_budget(0);
                    let p = if sync { p.with_sync_mirror() } else { p };
                    let got = if eager { p.with_eager_split() } else { p }.find();
                    assert_eq!(got, want, "size={r}x{c} threads={threads} sync_mirror={sync} eager_split={eager} dict={words:?}");
                }
            }
        }
    }
    assert!(some > 0 && none > 0, "want both outcomes: some={some} none={none}");
}

/// Symmetric word squares (`squares`): must equal the number of naive's square grids whose rows
/// equal their columns. Random dictionaries plus ENABLE 2×2..4×4.
#[test]
fn squares_match_naive_symmetric_grids() {
    let symmetric = |g: &Grid| (0..g.len()).all(|i| g[i].bytes().eq(g.iter().map(|row| row.as_bytes()[i])));
    let mut seed: u64 = 777;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as usize
    };
    for n in [2usize, 3, 4, 5] {
        for _ in 0..10 {
            let words: Vec<String> = (0..12).map(|_| (0..n).map(|_| (b'a' + (next() % 2) as u8) as char).collect()).collect();
            let d = Dict::from_words(words.iter().map(String::as_str));
            let want = all("naive", &d, n, n).iter().filter(|g| symmetric(g)).count() as u64;
            for threads in [1, 0] {
                let (got, _) = wordrect::squares::Squares::new(n, d.words(n)).count(threads);
                assert_eq!(got, want, "n={n} threads={threads} dict={words:?}");
            }
        }
    }
    let d = Dict::load("dicts/enable.txt".as_ref()).unwrap();
    for n in [2usize, 3] {
        let want = all("naive", &d, n, n).iter().filter(|g| symmetric(g)).count() as u64;
        assert_eq!(wordrect::squares::Squares::new(n, d.words(n)).count(0).0, want, "enable n={n}");
    }
}
