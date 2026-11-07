//! Minimized trie (DAWG / DAFSA) for words of one length, same API as `Trie`.
//!
//! Two trie nodes with the same set of suffixes are merged, so a node no longer
//! identifies a unique prefix, only "what can still follow". That shrinks the
//! automaton a lot and makes different partial grids reach *identical* column states,
//! which `par`'s memoized counting exploits.
//!
//! Layout: 8-byte nodes {mask, first} + a shared `edges` array. Because children are
//! shared, they can't be stored contiguously after their parent, so the child for
//! letter `l` is `edges[first + popcount(mask & ((1 << l) - 1))]` (one extra load vs `Trie`).

use std::collections::HashMap;

#[derive(Clone, Copy, Default)]
struct Node {
    mask: u32,
    first: u32,
}

pub struct Dawg {
    nodes: Vec<Node>,
    edges: Vec<u32>,
}

impl Dawg {
    /// `words` must be lowercase `a–z`, all of the same length.
    pub fn new(words: &[String]) -> Dawg {
        // 1. plain pointer trie
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
        // 2. minimize bottom-up: class of a node = its (letter, child class) list.
        //    Children always have larger indices than their parent, so a reverse scan is bottom-up.
        let mut class = vec![0u32; tmp.len()];
        let mut sig_to_class: HashMap<Vec<u32>, u32> = HashMap::new();
        let mut class_children: Vec<Vec<(u8, u32)>> = Vec::new();
        for n in (0..tmp.len()).rev() {
            let kids: Vec<(u8, u32)> = (0..26u8)
                .filter(|&l| tmp[n][l as usize] != 0)
                .map(|l| (l, class[tmp[n][l as usize] as usize]))
                .collect();
            let sig: Vec<u32> = kids.iter().map(|&(l, c)| (c << 5) | l as u32).collect();
            let next = class_children.len() as u32;
            let c = *sig_to_class.entry(sig).or_insert_with(|| {
                class_children.push(kids);
                next
            });
            class[n] = c;
        }
        // 3. BFS renumbering of classes from the root's class
        let root = class[0];
        let mut id = vec![u32::MAX; class_children.len()];
        let mut queue = vec![root];
        id[root as usize] = 0;
        let mut i = 0;
        while i < queue.len() {
            for &(_, c) in &class_children[queue[i] as usize] {
                if id[c as usize] == u32::MAX {
                    id[c as usize] = queue.len() as u32;
                    queue.push(c);
                }
            }
            i += 1;
        }
        let mut nodes = Vec::with_capacity(queue.len());
        let mut edges = Vec::new();
        for &c in &queue {
            let kids = &class_children[c as usize];
            let mask = kids.iter().fold(0u32, |m, &(l, _)| m | 1 << l);
            nodes.push(Node { mask, first: edges.len() as u32 });
            edges.extend(kids.iter().map(|&(_, k)| id[k as usize]));
        }
        Dawg { nodes, edges }
    }

    #[inline(always)]
    pub fn mask(&self, node: u32) -> u32 {
        self.nodes[node as usize].mask
    }

    /// Child for `letter`; only valid if bit `letter` is set in `mask(node)`.
    #[inline(always)]
    pub fn child(&self, node: u32, letter: usize) -> u32 {
        let n = self.nodes[node as usize];
        self.edges[(n.first + (n.mask & ((1u32 << letter) - 1)).count_ones()) as usize]
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes[0].mask == 0
    }
}
