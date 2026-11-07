//! Per-length trie with a 26-bit child mask on every node (README §6), compact layout (§8 Level 2).
//!
//! All words in one trie have the same length, so reaching depth `L` means a
//! complete word: no terminal flags needed.
//!
//! Layout: 16 bytes per node (mask, first child, two look-ahead masks). Nodes are stored in BFS order, and the children of a
//! node are contiguous and sorted by letter, so the child for letter `l` is
//! `first + popcount(mask & ((1 << l) - 1))` — no 26-entry child table.

#[derive(Clone, Copy, Default)]
pub struct Node {
    /// Bit `l` set <=> child for letter `l` exists.
    pub mask: u32,
    /// Index of the first (lowest-letter) child. Meaningless when `mask == 0`.
    pub first: u32,
    /// Look-ahead: union of the children's masks = letters some completion has one position
    /// further. Lets a search prune a letter whose row can't match the next-but-one column.
    pub next: u32,
    /// Same, two positions further.
    pub next2: u32,
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
        // 2. renumbering, sibling groups in depth-first (pre-order) order: a node's children
        //    are contiguous (so child = first + rank) and a whole subtree sits in one region of
        //    the array (cache locality along search paths).
        let mut nodes = vec![Node::default(); tmp.len()];
        fn place(tmp: &[[u32; 26]], nodes: &mut [Node], old: usize, new: usize, next: &mut usize) {
            let kids: Vec<(usize, usize)> = tmp[old].iter().enumerate().filter(|&(_, &c)| c != 0).map(|(l, &c)| (l, c as usize)).collect();
            let first = *next;
            *next += kids.len();
            let mask = kids.iter().fold(0u32, |m, &(l, _)| m | 1 << l);
            nodes[new] = Node { mask, first: first as u32, next: 0, next2: 0 };
            for (j, &(_, c)) in kids.iter().enumerate() {
                place(tmp, nodes, c, first + j, next);
            }
        }
        let mut next = 1;
        place(&tmp, &mut nodes, 0, 0, &mut next);
        // children come after their parent: fill `next` bottom-up
        for i in (0..nodes.len()).rev() {
            let n = nodes[i];
            nodes[i].next = (0..n.mask.count_ones()).fold(0, |a, j| a | nodes[(n.first + j) as usize].mask);
            nodes[i].next2 = (0..n.mask.count_ones()).fold(0, |a, j| a | nodes[(n.first + j) as usize].next);
        }
        Trie { nodes }
    }

    #[inline(always)]
    pub fn mask(&self, node: u32) -> u32 {
        self.nodes[node as usize].mask
    }

    /// Child for `letter`; only valid if bit `letter` is set in `mask(node)`.
    #[inline(always)]
    pub fn child(&self, node: u32, letter: usize) -> u32 {
        let n = self.nodes[node as usize];
        n.first + (n.mask & ((1u32 << letter) - 1)).count_ones()
    }

    /// The node itself (by value), for searches that carry nodes instead of indices.
    #[inline(always)]
    pub fn node(&self, node: u32) -> Node {
        self.nodes[node as usize]
    }

    /// Child node (by value) of `n` for `letter`; only valid if bit `letter` is set in `n.mask`.
    #[inline(always)]
    pub fn step(&self, n: Node, letter: u32) -> Node {
        self.nodes[(n.first + (n.mask & ((1u32 << letter) - 1)).count_ones()) as usize]
    }

    /// Index of the child of `n` for `letter` (same rule as `step`).
    #[inline(always)]
    pub fn child_idx(n: Node, letter: u32) -> u32 {
        n.first + (n.mask & ((1u32 << letter) - 1)).count_ones()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes[0].mask == 0
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
        CTrie { nodes: t.nodes.iter().map(|n| CNode { mask: n.mask, first: n.first }).collect() }
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

    #[inline(always)]
    pub fn step(&self, n: CNode, letter: u32) -> CNode {
        self.nodes[(n.first + (n.mask & ((1u32 << letter) - 1)).count_ones()) as usize]
    }
}
