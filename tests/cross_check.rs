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
                validate(g, &dict).unwrap();
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
            validate(&g, &dict).unwrap();
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
