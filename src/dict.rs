//! Loading and cleaning dictionaries.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::Path;

pub struct Dict {
    /// `by_len[n]` = sorted, deduplicated words of length `n`.
    pub by_len: Vec<Vec<String>>,
    pub total: usize,
}

impl Dict {
    pub fn load(path: &Path) -> io::Result<Dict> {
        Ok(Dict::from_words(fs::read_to_string(path)?.lines()))
    }

    /// Lowercase, keep only words made entirely of `a–z`, deduplicate, split by length.
    pub fn from_words<'a, I: IntoIterator<Item = &'a str>>(words: I) -> Dict {
        let set: BTreeSet<String> = words
            .into_iter()
            .map(|w| w.trim().to_ascii_lowercase())
            .filter(|w| !w.is_empty() && w.bytes().all(|b| b.is_ascii_lowercase()))
            .collect();
        let max = set.iter().map(String::len).max().unwrap_or(0);
        let mut by_len = vec![Vec::new(); max + 1];
        for w in &set {
            by_len[w.len()].push(w.clone());
        }
        Dict { by_len, total: set.len() }
    }

    pub fn words(&self, len: usize) -> &[String] {
        self.by_len.get(len).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn max_len(&self) -> usize {
        self.by_len.len().saturating_sub(1)
    }
}

/// 64-bit FNV-1a of the raw file: a cheap fingerprint printed with every result.
/// (The official SHA-256 lives in dicts/HASHES.txt.)
pub fn fingerprint(path: &Path) -> io::Result<String> {
    let bytes = fs::read(path)?;
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    Ok(format!("{h:016x}"))
}
