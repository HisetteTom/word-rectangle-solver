//! The fast loader (sorted lists, length filter, parallel chunks) must give exactly what the
//! general path gives, on every kind of input.
use wordrect::dict::Dict;
use wordrect::trie::{CTrie, Trie};

fn tmp(name: &str, text: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("wordrect-test-{}-{name}", std::process::id()));
    std::fs::write(&p, text).unwrap();
    p
}

fn enable() -> String {
    std::fs::read_to_string("dicts/enable.txt").unwrap()
}

#[test]
fn fast_load_matches_general_parse() {
    let text = enable();
    let want = Dict::from_words(text.lines());
    let d = Dict::load("dicts/enable.txt".as_ref()).unwrap();
    assert_eq!(d.total, want.total);
    for n in 0..=want.max_len() {
        assert_eq!(d.words(n), want.words(n), "len {n}");
    }
}

#[test]
fn messy_lists_load_the_same() {
    let text = enable();
    let want = Dict::from_words(text.lines());
    let lines: Vec<&str> = text.lines().collect();
    // reversed (unsorted), with duplicates, uppercase, CRLF, blanks and junk: all through a file
    let reversed: String = lines.iter().rev().map(|w| format!("{w}\n")).collect();
    let dups: String = lines.iter().flat_map(|w| [w, w]).map(|w| format!("{w}\n")).collect();
    let upper: String = lines.iter().map(|w| format!("{}\n", w.to_uppercase())).collect();
    let crlf: String = lines.iter().map(|w| format!("{w}\r\n")).collect();
    let junk: String = lines.iter().map(|w| format!("{w}\n\ndon't\n")).collect();
    for (name, t) in [("reversed", reversed), ("dups", dups), ("upper", upper), ("crlf", crlf), ("junk", junk)] {
        let p = tmp(name, &t);
        let d = Dict::load(&p).unwrap();
        assert_eq!(d.total, want.total, "{name}");
        for n in 0..=want.max_len() {
            assert_eq!(d.words(n), want.words(n), "{name} len {n}");
        }
        let k = Dict::load_only(&p, Some(&[5, 7])).unwrap();
        assert_eq!(k.total, want.total, "{name} total with a length filter");
        assert_eq!(k.words(5), want.words(5), "{name}");
        assert_eq!(k.words(7), want.words(7), "{name}");
        assert!(k.words(6).is_empty(), "{name}: unkept length must be empty");
        std::fs::remove_file(p).unwrap();
    }
}

#[test]
fn small_files_and_chunk_edges() {
    for (text, total, threes) in [
        ("", 0, vec![]),
        ("cat", 1, vec!["cat"]),
        ("cat\n", 1, vec!["cat"]),
        ("ape\ncat\ndog\n", 3, vec!["ape", "cat", "dog"]),
        ("dog\ncat\ncat\n", 2, vec!["cat", "dog"]),
        ("ab\nape\nb\ncat\n", 4, vec!["ape", "cat"]),
    ] {
        let p = tmp("small", text);
        let d = Dict::load(&p).unwrap();
        assert_eq!(d.total, total, "{text:?}");
        assert_eq!(d.words(3), threes, "{text:?}");
        std::fs::remove_file(p).unwrap();
    }
    // a big sorted file whose chunks meet out of order: must fall back, not merge blindly
    let text = enable();
    let mut lines: Vec<&str> = text.lines().collect();
    let mid = lines.len() / 2;
    lines.swap(10, mid);
    let p = tmp("swapped", &lines.join("\n"));
    let want = Dict::from_words(text.lines());
    let d = Dict::load(&p).unwrap();
    assert_eq!(d.total, want.total);
    for n in 0..=want.max_len() {
        assert_eq!(d.words(n), want.words(n), "len {n}");
    }
    std::fs::remove_file(p).unwrap();
}

#[test]
fn tries_spell_back_their_words() {
    let d = Dict::load("dicts/enable.txt".as_ref()).unwrap();
    for n in 1..=12 {
        let w = d.words(n);
        let t = Trie::new(w);
        assert_eq!(t.words(), w, "len {n}");
        assert_eq!(CTrie::new(&t).words(), w, "len {n}");
        // unsorted input with duplicates builds the same trie
        let mut messy: Vec<String> = w.iter().rev().cloned().collect();
        messy.extend(w.iter().take(20).cloned());
        assert_eq!(Trie::new(&messy).words(), w, "len {n}");
    }
}
