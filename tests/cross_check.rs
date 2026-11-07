//! Every optimized solver must agree exactly with the naive reference.

use std::collections::HashSet;
use wordrect::Solver;
use wordrect::dict::Dict;
use wordrect::naive::Naive;
use wordrect::search::TrieSearch;
use wordrect::validate::validate;

fn sorted(mut v: Vec<Vec<String>>) -> Vec<Vec<String>> {
    v.sort();
    v
}

fn naive_all(d: &Dict, r: usize, c: usize) -> Vec<Vec<String>> {
    sorted(Naive::new(r, c, d.words(c), d.words(r)).all())
}

fn trie_all(d: &Dict, r: usize, c: usize) -> Vec<Vec<String>> {
    sorted(TrieSearch::new(r, c, d.words(c), d.words(r)).all())
}

#[test]
fn trie_matches_naive_on_pseudo_random_dicts() {
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
            assert_eq!(trie_all(&d, r, c), naive_all(&d, r, c), "dict={words:?} size={r}x{c}");
        }
    }
}

#[test]
fn trie_matches_naive_on_enable_small_sizes() {
    let d = Dict::load("dicts/enable.txt".as_ref()).unwrap();
    let all: HashSet<String> = d.by_len.iter().flatten().cloned().collect();
    for (r, c) in [(2, 2), (2, 3), (3, 2)] {
        let t = trie_all(&d, r, c);
        assert!(!t.is_empty());
        assert_eq!(t, naive_all(&d, r, c), "size={r}x{c}");
        for g in &t {
            validate(g, &all).unwrap();
        }
    }
}

#[test]
fn trie_finds_same_first_solution_as_naive() {
    // Both explore in lexicographic order, so the first grid must be identical.
    let d = Dict::load("dicts/enable.txt".as_ref()).unwrap();
    for (r, c) in [(3, 3), (3, 4), (4, 4)] {
        let n = Naive::new(r, c, d.words(c), d.words(r)).find();
        let t = TrieSearch::new(r, c, d.words(c), d.words(r)).find();
        assert_eq!(t, n, "size={r}x{c}");
    }
}

#[test]
fn trie_empty_and_tiny() {
    let d = Dict::from_words(["cat", "ape", "ten"]);
    assert_eq!(trie_all(&d, 3, 3), vec![vec!["cat".to_string(), "ape".into(), "ten".into()]]);
    let d = Dict::from_words(["cat", "dog"]);
    assert!(trie_all(&d, 3, 3).is_empty());
    assert!(trie_all(&d, 5, 5).is_empty()); // no words of that length at all
}
