use std::collections::HashSet;
use wordrect::dict::Dict;
use wordrect::naive::Naive;
use wordrect::validate::validate;

fn set(words: &[&str]) -> HashSet<String> {
    words.iter().map(|s| s.to_string()).collect()
}

/// Brute force over every combination of row words: completely independent of the solver.
fn brute_force(rows: usize, cols: usize, words: &[&str]) -> Vec<Vec<String>> {
    let d = Dict::from_words(words.iter().copied());
    let dict = set(words);
    let rw = d.words(cols);
    let mut out = Vec::new();
    let total = rw.len().pow(rows as u32);
    for mut idx in 0..total {
        let mut g = Vec::new();
        for _ in 0..rows {
            g.push(rw[idx % rw.len()].clone());
            idx /= rw.len();
        }
        if validate(&g, &dict).is_ok() {
            out.push(g);
        }
    }
    out.sort();
    out
}

fn naive_all(rows: usize, cols: usize, words: &[&str]) -> Vec<Vec<String>> {
    let d = Dict::from_words(words.iter().copied());
    let mut s = Naive::new(rows, cols, d.words(cols), d.words(rows));
    let mut v = s.all();
    v.sort();
    v
}

#[test]
fn cat_ape_ten() {
    let words = ["cat", "ape", "ten"];
    let sols = naive_all(3, 3, &words);
    assert_eq!(sols, vec![vec!["cat".to_string(), "ape".into(), "ten".into()]]);
}

#[test]
fn no_solution() {
    assert!(naive_all(3, 3, &["cat", "dog", "pig"]).is_empty());
}

#[test]
fn rectangle_2x3() {
    // rows: 3-letter words, cols: 2-letter words
    let words = ["ab", "ba", "aa", "aba", "bab", "aab"];
    assert_eq!(naive_all(2, 3, &words), brute_force(2, 3, &words));
    assert!(!naive_all(2, 3, &words).is_empty());
}

#[test]
fn matches_brute_force_on_pseudo_random_dicts() {
    // Small LCG so the test is deterministic and dependency-free.
    let mut seed: u64 = 12345;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as usize
    };
    for _ in 0..30 {
        let mut words = Vec::new();
        for _ in 0..12 {
            let len = 2 + next() % 2;
            let w: String = (0..len).map(|_| (b'a' + (next() % 3) as u8) as char).collect();
            words.push(w);
        }
        let refs: Vec<&str> = words.iter().map(String::as_str).collect();
        for (r, c) in [(2, 2), (2, 3), (3, 2), (3, 3)] {
            assert_eq!(naive_all(r, c, &refs), brute_force(r, c, &refs), "dict={refs:?} size={r}x{c}");
        }
    }
}

#[test]
fn transpose_counts_match() {
    let words = ["ab", "ba", "aa", "bb", "aba", "bab", "aab", "abb", "bba"];
    assert_eq!(naive_all(2, 3, &words).len(), naive_all(3, 2, &words).len());
}

#[test]
fn every_solution_validates() {
    let words = ["ab", "ba", "aa", "bb", "aba", "bab", "aab", "abb", "bba", "aaa"];
    let dict = set(&words);
    for g in naive_all(3, 3, &words) {
        validate(&g, &dict).unwrap();
    }
}
