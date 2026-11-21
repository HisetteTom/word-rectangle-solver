//! Per-length trie with a 26-bit child mask on every node (README §6), compact layout (§8 Level 2).
//!
//! All words in one trie have the same length, so reaching depth `L` means a
//! complete word: no terminal flags needed.
//!
//! Layout: 12 bytes per node (three letter masks; the first-child index is packed into their
//! spare top bits). Nodes are stored in BFS order, and the children of a
//! node are contiguous and sorted by letter, so the child for letter `l` is
//! `first + popcount(mask & ((1 << l) - 1))` — no 26-entry child table.

/// The 26 letter bits of a mask.
pub const LETTERS: u32 = (1 << 26) - 1;
/// Tries are limited to 2^18 nodes (18 index bits = 3 × the 6 spare bits of the masks).
const MAX_NODES: usize = 1 << 18;

/// 12-byte node. Bits 0..26 of each field are a letter mask; bits 26..32 of the three fields hold
/// the first-child index (6 bits each, `mask` = low bits). Users AND the masks with clean 26-bit
/// masks (or `LETTERS`) before testing them, and use `first()` for the index.
#[derive(Clone, Copy, Default)]
pub struct Node {
    /// Bits 0..26: bit `l` set <=> child for letter `l` exists.
    pub mask: u32,
    /// Look-ahead: union of the children's masks = letters some completion has one position
    /// further. Lets a search prune a letter whose row can't match the next-but-one column.
    pub next: u32,
    /// Same, two positions further.
    pub next2: u32,
}

impl Node {
    fn pack(mask: u32, first: u32, next: u32, next2: u32) -> Node {
        Node { mask: mask | (first & 63) << 26, next: next | (first >> 6 & 63) << 26, next2: next2 | (first >> 12 & 63) << 26 }
    }

    /// Index of the first (lowest-letter) child. Meaningless when the node has no children.
    #[inline(always)]
    pub fn first(self) -> u32 {
        (self.mask >> 26) | (self.next >> 26) << 6 | (self.next2 >> 26) << 12
    }
}

pub struct Trie {
    nodes: Vec<Node>,
}

impl Trie {
    /// `words` must be lowercase `a–z` (guaranteed by `Dict`).
    pub fn new(words: &[String]) -> Trie {
        // 1. plain pointer trie (build-time only)
        let mut tmp: Vec<[u32; 26]> = vec![[0; 26]];
        for w in words {
            let mut n = 0;
            for b in w.bytes() {
                let l = (b - b'a') as usize;
                if tmp[n][l] == 0 {
                    tmp[n][l] = tmp.len() as u32;
                    tmp.push([0; 26]);
                }
                n = tmp[n][l] as usize;
            }
        }
        // 2. BFS renumbering: queue position = new index
        let mut queue = vec![0usize];
        // (mask, first, next, next2), unpacked while building
        let mut raw: Vec<(u32, u32, u32, u32)> = Vec::with_capacity(tmp.len());
        let mut i = 0;
        while i < queue.len() {
            let old = queue[i];
            let first = queue.len() as u32;
            let mut mask = 0;
            for (l, &c) in tmp[old].iter().enumerate() {
                if c != 0 {
                    mask |= 1 << l;
                    queue.push(c as usize);
                }
            }
            raw.push((mask, first, 0, 0));
            i += 1;
        }
        assert!(raw.len() <= MAX_NODES, "trie too big for 18-bit child indices: {} nodes", raw.len());
        // children come after their parent in BFS order: fill `next` bottom-up
        for i in (0..raw.len()).rev() {
            let (mask, first, ..) = raw[i];
            let kids = first as usize..(first + mask.count_ones()) as usize;
            raw[i].2 = raw[kids.clone()].iter().fold(0, |a, k| a | k.0);
            raw[i].3 = raw[kids].iter().fold(0, |a, k| a | k.2);
        }
        // a childless node's `first` may point one past the end: store 0 (never followed)
        let nodes = raw.iter().map(|&(m, f, n, n2)| Node::pack(m, if m == 0 { 0 } else { f }, n, n2)).collect();
        Trie { nodes }
    }

    #[inline(always)]
    pub fn mask(&self, node: u32) -> u32 {
        self.nodes[node as usize].mask & LETTERS
    }

    /// Child for `letter`; only valid if bit `letter` is set in `mask(node)`.
    #[inline(always)]
    pub fn child(&self, node: u32, letter: usize) -> u32 {
        let n = self.nodes[node as usize];
        n.first() + (n.mask & ((1u32 << letter) - 1)).count_ones()
    }

    /// The node itself (by value), for searches that carry nodes instead of indices.
    #[inline(always)]
    pub fn node(&self, node: u32) -> Node {
        self.nodes[node as usize]
    }

    /// Child node (by value) of `n` for `letter`; only valid if bit `letter` is set in `n.mask`.
    #[inline(always)]
    pub fn step(&self, n: Node, letter: u32) -> Node {
        self.nodes[Self::child_idx(n, letter) as usize]
    }

    /// Index of the child of `n` for `letter` (same rule as `step`).
    #[inline(always)]
    pub fn child_idx(n: Node, letter: u32) -> u32 {
        n.first() + (n.mask & ((1u32 << letter) - 1)).count_ones()
    }

    /// All nodes (BFS order), for kernels that index them directly.
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes[0].mask & LETTERS == 0
    }
}

/// Compact 8-byte node (no look-ahead masks), for tries that only need `mask` + child steps.
#[derive(Clone, Copy, Default)]
pub struct CNode {
    pub mask: u32,
    pub first: u32,
}

/// Same trie as `Trie` (same indices) with 8-byte nodes: half the cache footprint.
/// Column tries use it: their look-ahead is never read.
pub struct CTrie {
    nodes: Vec<CNode>,
}

impl CTrie {
    pub fn new(t: &Trie) -> CTrie {
        CTrie { nodes: t.nodes.iter().map(|n| CNode { mask: n.mask & LETTERS, first: n.first() }).collect() }
    }

    #[inline(always)]
    pub fn mask(&self, node: u32) -> u32 {
        self.nodes[node as usize].mask
    }

    #[inline(always)]
    pub fn child(&self, node: u32, letter: usize) -> u32 {
        let n = self.nodes[node as usize];
        n.first + (n.mask & ((1u32 << letter) - 1)).count_ones()
    }

    #[inline(always)]
    pub fn node(&self, node: u32) -> CNode {
        self.nodes[node as usize]
    }

    /// All nodes (same indices as the `Trie` it came from).
    pub fn nodes(&self) -> &[CNode] {
        &self.nodes
    }

    #[inline(always)]
    pub fn step(&self, n: CNode, letter: u32) -> CNode {
        self.nodes[(n.first + (n.mask & ((1u32 << letter) - 1)).count_ones()) as usize]
    }
}
