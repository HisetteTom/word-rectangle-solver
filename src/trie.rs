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
    /// `words` must be lowercase `a–z` (guaranteed by `Dict`), all of one length.
    ///
    /// BFS order of a trie whose children are sorted by letter is: depth by depth, prefixes in
    /// sorted order. So with the words sorted, the nodes at depth `d` are just the distinct
    /// `d`-letter prefixes in order, and one pass builds the final layout directly.
    pub fn new(words: &[String]) -> Trie {
        let mut sorted: Vec<&[u8]> = words.iter().map(|w| w.as_bytes()).collect();
        if !sorted.windows(2).all(|p| p[0] < p[1]) {
            sorted.sort_unstable();
            sorted.dedup();
        }
        let len = sorted.first().map_or(0, |w| w.len());
        // lcp[i] = common prefix length of word i with word i-1: word i starts a new node at
        // every depth d > lcp[i] (word 0 at every depth)
        let lcp: Vec<usize> = (0..sorted.len())
            .map(|i| if i == 0 { 0 } else { sorted[i].iter().zip(sorted[i - 1]).take_while(|(a, b)| a == b).count() })
            .collect();
        // level sizes, then the index of the first node of each depth
        let mut start = vec![0usize; len + 2];
        start[1] = 1; // the root
        for d in 1..=len {
            start[d + 1] = start[d] + lcp.iter().filter(|&&l| l < d).count();
        }
        let total = start[len + 1];
        // leaves keep the old builder's `first` (the node count): it is never followed
        let mut nodes = vec![Node { mask: 0, first: total as u32, next: 0, next2: 0 }; total.max(1)];
        let mut cur = vec![0usize; len + 1]; // current node at each depth
        let mut made = start.clone(); // next free index at each depth
        for (w, &l) in sorted.iter().zip(&lcp) {
            let new = (l + 1).min(len + 1)..len + 1; // depths where this word starts a node
            cur[new.clone()].copy_from_slice(&made[new.clone()]);
            made[new].iter_mut().for_each(|m| *m += 1);
            for d in 0..len {
                let n = &mut nodes[cur[d]];
                if n.mask == 0 {
                    n.first = cur[d + 1] as u32; // first child = the first one made under it
                }
                n.mask |= 1 << (w[d] - b'a');
            }
        }
        // children come after their parent in BFS order: fill `next` bottom-up
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

    /// All nodes (BFS order), for kernels that index them directly.
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes[0].mask == 0
    }

    /// The words it was built from, sorted and deduplicated.
    pub fn words(&self) -> Vec<String> {
        spell(|n| (self.nodes[n as usize].mask, self.nodes[n as usize].first))
    }
}

/// Every word of a trie, in sorted order, from its `(mask, first)` pairs.
fn spell(node: impl Fn(u32) -> (u32, u32)) -> Vec<String> {
    fn go(node: &impl Fn(u32) -> (u32, u32), n: u32, word: &mut Vec<u8>, out: &mut Vec<String>) {
        let (mask, first) = node(n);
        if mask == 0 {
            out.push(String::from_utf8(word.clone()).unwrap());
            return;
        }
        let mut m = mask;
        let mut child = first;
        while m != 0 {
            word.push(b'a' + m.trailing_zeros() as u8);
            go(node, child, word, out);
            word.pop();
            m &= m - 1;
            child += 1;
        }
    }
    let mut out = Vec::new();
    if node(0).0 != 0 {
        go(&node, 0, &mut Vec::new(), &mut out);
    }
    out
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

    /// The words it was built from, sorted and deduplicated.
    pub fn words(&self) -> Vec<String> {
        spell(|n| (self.nodes[n as usize].mask, self.nodes[n as usize].first))
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
