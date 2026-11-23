//! Loading and cleaning dictionaries.

use std::borrow::Cow;
use std::fs;
use std::io;
use std::path::Path;

pub struct Dict {
    /// `by_len[n]` = sorted, deduplicated words of length `n` (empty or missing for lengths not kept).
    pub by_len: Vec<Vec<String>>,
    /// Distinct valid words in the whole list, kept or not.
    pub total: usize,
}

impl Dict {
    pub fn load(path: &Path) -> io::Result<Dict> {
        Dict::load_only(path, None)
    }

    /// Like `load`, but only words whose length is in `keep` are kept (all when `None`);
    /// `total` still counts the whole list.
    pub fn load_only(path: &Path, keep: Option<&[usize]>) -> io::Result<Dict> {
        let text = fs::read_to_string(path)?;
        Ok(Dict::parse_sorted(&text, keep).unwrap_or_else(|| Dict::parse(text.lines(), keep)))
    }

    /// Fast path for a list of clean (`a–z`), sorted, duplicate-free lines (the usual case): no
    /// sorting, only kept lengths are copied, big files scanned in parallel chunks.
    /// `None` = the list needs `parse`.
    fn parse_sorted(text: &str, keep: Option<&[usize]>) -> Option<Dict> {
        let parts = if text.len() < 1 << 18 { 1 } else { std::thread::available_parallelism().map_or(1, |n| n.get().min(8)) };
        Dict::parse_sorted_in(text, keep, parts)
    }

    fn parse_sorted_in(text: &str, keep: Option<&[usize]>, parts: usize) -> Option<Dict> {
        let bytes = text.as_bytes();
        // chunk boundaries just after a '\n'
        let mut cuts = vec![0];
        for i in 1..parts {
            let at = (bytes.len() * i / parts).max(*cuts.last().unwrap());
            cuts.push(bytes[at..].iter().position(|&b| b == b'\n').map_or(bytes.len(), |p| at + p + 1));
        }
        cuts.push(bytes.len());
        let scans: Vec<Option<Scan>> = std::thread::scope(|s| {
            let hs: Vec<_> = cuts.windows(2).skip(1).map(|c| s.spawn(move || scan(&bytes[c[0]..c[1]], keep))).collect();
            let mut v = vec![scan(&bytes[cuts[0]..cuts[1]], keep)];
            v.extend(hs.into_iter().map(|h| h.join().unwrap()));
            v
        });
        let mut by_len: Vec<Vec<String>> = Vec::new();
        let (mut total, mut last): (usize, &[u8]) = (0, &[]);
        for sc in scans {
            let sc = sc?;
            if sc.total == 0 {
                continue;
            }
            if total > 0 && last >= sc.first {
                return None; // not sorted across chunks
            }
            if by_len.len() < sc.by_len.len() {
                by_len.resize(sc.by_len.len(), Vec::new());
            }
            for (all, part) in by_len.iter_mut().zip(sc.by_len) {
                all.extend(part);
            }
            total += sc.total;
            last = sc.last;
        }
        Some(Dict { by_len, total })
    }

    /// Lowercase, keep only words made entirely of `a–z`, deduplicate, split by length.
    pub fn from_words<'a, I: IntoIterator<Item = &'a str>>(words: I) -> Dict {
        Dict::parse(words, None)
    }

    fn parse<'a, I: IntoIterator<Item = &'a str>>(words: I, keep: Option<&[usize]>) -> Dict {
        let mut all: Vec<Cow<str>> = words.into_iter().filter_map(clean).collect();
        all.sort_unstable();
        all.dedup();
        let max = all.iter().map(|w| w.len()).max().unwrap_or(0);
        let mut by_len = vec![Vec::new(); max + 1];
        for w in &all {
            if keep.is_none_or(|k| k.contains(&w.len())) {
                by_len[w.len()].push(w.to_string());
            }
        }
        Dict { by_len, total: all.len() }
    }

    pub fn words(&self, len: usize) -> &[String] {
        self.by_len.get(len).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn max_len(&self) -> usize {
        self.by_len.len().saturating_sub(1)
    }
}

/// One chunk of a clean, sorted list (see `Dict::parse_sorted`).
struct Scan<'a> {
    by_len: Vec<Vec<String>>,
    total: usize,
    first: &'a [u8],
    last: &'a [u8],
}

/// `None` = a line needs cleaning, or the chunk is not strictly sorted.
fn scan<'a>(chunk: &'a [u8], keep: Option<&[usize]>) -> Option<Scan<'a>> {
    let mut sc = Scan { by_len: Vec::new(), total: 0, first: &[], last: &[] };
    for line in chunk.split(|&b| b == b'\n') {
        if !line.iter().all(u8::is_ascii_lowercase) {
            if line.iter().any(|b| b.is_ascii_uppercase() || b.is_ascii_whitespace()) {
                return None; // needs trimming or lowercasing
            }
            continue; // a word with other characters: never valid
        }
        if line.is_empty() {
            continue;
        }
        if sc.total == 0 {
            sc.first = line;
        } else if sc.last >= line {
            return None; // not sorted (or a duplicate)
        }
        if keep.is_none_or(|k| k.contains(&line.len())) {
            if sc.by_len.len() <= line.len() {
                sc.by_len.resize(line.len() + 1, Vec::new());
            }
            sc.by_len[line.len()].push(String::from_utf8(line.to_vec()).unwrap());
        }
        sc.total += 1;
        sc.last = line;
    }
    Some(sc)
}

/// Trimmed, lowercased word, `None` if it is empty or not all `a–z`.
fn clean(w: &str) -> Option<Cow<'_, str>> {
    let w = w.trim();
    let w = if w.bytes().any(|b| b.is_ascii_uppercase()) { Cow::Owned(w.to_ascii_lowercase()) } else { Cow::Borrowed(w) };
    (!w.is_empty() && w.bytes().all(|b| b.is_ascii_lowercase())).then_some(w)
}

/// 64-bit FNV-1a of the raw file: a cheap fingerprint printed with every result.
/// (The official SHA-256 lives in dicts/HASHES.txt.)
pub fn fingerprint(path: &Path) -> io::Result<String> {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in fs::read(path)? {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    Ok(format!("{h:016x}"))
}

#[cfg(test)]
mod tests {
    use super::Dict;

    #[test]
    fn chunks_must_also_be_in_order_with_each_other() {
        // each chunk alone is sorted: only the check between chunks can see these
        for text in ["dog\ncat\n", "cat\ncat\n", "ape\ndog\ncat\nemu\n"] {
            for parts in 2..=4 {
                assert!(Dict::parse_sorted_in(text, None, parts).is_none(), "{text:?} in {parts} parts");
            }
        }
        for parts in 1..=6 {
            let d = Dict::parse_sorted_in("ape\ncat\ndog\nemu\n", Some(&[3]), parts).unwrap();
            assert_eq!((d.total, d.words(3)), (4, &["ape", "cat", "dog", "emu"].map(String::from)[..]), "{parts} parts");
        }
    }
}
