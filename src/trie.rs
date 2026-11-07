//! Per-length trie with a 26-bit child mask on every node (README §6).
//!
//! All words in one trie have the same length, so reaching depth `L` means a
//! complete word: no terminal flags needed.

#[derive(Clone, Default)]
struct Node {
    /// Child index per letter; 0 = missing (0 is the root, never a child).
    children: [u32; 26],
    /// Bit `l` set <=> child for letter `l` exists.
    mask: u32,
}

pub struct Trie {
    nodes: Vec<Node>,
}

impl Trie {
    /// `words` must be lowercase `a–z` (guaranteed by `Dict`).
    pub fn new(words: &[String]) -> Trie {
        let mut nodes = vec![Node::default()];
        for w in words {
            let mut n = 0;
            for b in w.bytes() {
                let l = (b - b'a') as usize;
                let child = nodes[n].children[l];
                n = if child != 0 {
                    child as usize
                } else {
                    let id = nodes.len();
                    nodes.push(Node::default());
                    nodes[n].children[l] = id as u32;
                    nodes[n].mask |= 1 << l;
                    id
                };
            }
        }
        Trie { nodes }
    }

    #[inline(always)]
    pub fn mask(&self, node: u32) -> u32 {
        self.nodes[node as usize].mask
    }

    #[inline(always)]
    pub fn child(&self, node: u32, letter: usize) -> u32 {
        self.nodes[node as usize].children[letter]
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes[0].mask == 0
    }
}
