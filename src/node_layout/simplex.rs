//! The network simplex method of Gansner, Koutsofios, North and Vo ("A technique for drawing
//! directed graphs", 1993), as Graphviz dot and dagre use it: whole numbers r for the nodes of a
//! directed graph without loops, each edge tail → head keeping r[head] − r[tail] ≥ minlen, with the
//! least Σ weight × (r[head] − r[tail]). The layout uses it twice: for the columns (every wire as
//! short as it can be) and for the heights (Gansner's auxiliary graph, every wire as level as it can
//! be, with the sockets where they are).

use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Copy, Debug)]
pub struct Edge {
    pub tail: usize,
    pub head: usize,
    pub minlen: i64,
    pub weight: i64,
}

/// Exchanges allowed per node and edge, against a cycle the method can in theory fall into. What
/// it has by then is still a valid answer, only maybe not the best.
const EXCHANGES_PER_ITEM: usize = 50;

/// The best values, the lowest 0. The edges must not make a loop.
pub fn solve(n: usize, edges: &[Edge]) -> Vec<i64> {
    if n == 0 {
        return Vec::new();
    }
    // Parallel edges as one: weights added, the longest minlen.
    let mut merged: BTreeMap<(usize, usize), (i64, i64)> = BTreeMap::new();
    for e in edges.iter().filter(|e| e.tail != e.head && e.tail < n && e.head < n) {
        let m = merged.entry((e.tail, e.head)).or_insert((e.minlen, 0));
        m.0 = m.0.max(e.minlen);
        m.1 += e.weight;
    }
    let mut g: Vec<Edge> = merged.into_iter().map(|((tail, head), (minlen, weight))| Edge { tail, head, minlen, weight }).collect();
    let rank = longest_path(n, &g);
    join_pieces(n, &mut g, &rank);
    let mut s = Simplex::new(n, g, rank);
    s.feasible_tree();
    s.run();
    let lo = s.rank.iter().copied().min().unwrap_or(0);
    s.rank.iter().map(|r| r - lo).collect()
}

/// A first answer that keeps every edge: each node as low as its edges in allow.
fn longest_path(n: usize, g: &[Edge]) -> Vec<i64> {
    let mut waiting = vec![0usize; n];
    let mut out = vec![Vec::new(); n];
    for (k, e) in g.iter().enumerate() {
        waiting[e.head] += 1;
        out[e.tail].push(k);
    }
    let mut rank = vec![0i64; n];
    let mut queue: VecDeque<usize> = (0..n).filter(|&v| waiting[v] == 0).collect();
    while let Some(v) = queue.pop_front() {
        for &k in &out[v] {
            let e = g[k];
            rank[e.head] = rank[e.head].max(rank[v] + e.minlen);
            waiting[e.head] -= 1;
            if waiting[e.head] == 0 {
                queue.push_back(e.head);
            }
        }
    }
    debug_assert!(waiting.iter().all(|&w| w == 0), "network simplex: the edges make a loop");
    rank
}

/// Separate pieces joined to node 0 by weightless edges, tight as they stand: they add nothing to
/// the sum and only keep the pieces where they are relative to each other.
fn join_pieces(n: usize, g: &mut Vec<Edge>, rank: &[i64]) {
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    let mut p: Vec<usize> = (0..n).collect();
    for e in g.iter() {
        let (a, b) = (find(&mut p, e.tail), find(&mut p, e.head));
        p[a] = b;
    }
    for v in 1..n {
        let (a, b) = (find(&mut p, 0), find(&mut p, v));
        if a != b {
            g.push(Edge { tail: 0, head: v, minlen: rank[v] - rank[0], weight: 0 });
            p[b] = a;
        }
    }
}

struct Simplex {
    n: usize,
    g: Vec<Edge>,
    rank: Vec<i64>,
    /// The edges at each node.
    at: Vec<Vec<usize>>,
    /// Which edges are in the spanning tree.
    tree: Vec<bool>,
    parent: Vec<usize>,
    /// The tree edge to the parent.
    up: Vec<usize>,
    /// Postorder number from the root, and the lowest one under each node.
    lim: Vec<usize>,
    low: Vec<usize>,
    /// Postorder, the root last.
    post: Vec<usize>,
    cut: Vec<i64>,
}

impl Simplex {
    fn new(n: usize, g: Vec<Edge>, rank: Vec<i64>) -> Self {
        let mut at = vec![Vec::new(); n];
        for (k, e) in g.iter().enumerate() {
            at[e.tail].push(k);
            at[e.head].push(k);
        }
        let m = g.len();
        Simplex { n, g, rank, at, tree: vec![false; m], parent: vec![usize::MAX; n], up: vec![usize::MAX; n], lim: vec![0; n], low: vec![0; n], post: Vec::new(), cut: vec![0; m] }
    }

    fn slack(&self, k: usize) -> i64 {
        let e = self.g[k];
        self.rank[e.head] - self.rank[e.tail] - e.minlen
    }

    fn other(&self, k: usize, v: usize) -> usize {
        let e = self.g[k];
        if e.tail == v {
            e.head
        } else {
            e.tail
        }
    }

    /// A spanning tree of tight edges: grown from node 0, and where it cannot grow, everything in
    /// it is shifted to make the nearest edge out of it tight.
    fn feasible_tree(&mut self) {
        loop {
            let mut inside = vec![false; self.n];
            self.tree.iter_mut().for_each(|t| *t = false);
            inside[0] = true;
            let mut todo = vec![0usize];
            let mut count = 1;
            while let Some(v) = todo.pop() {
                for i in 0..self.at[v].len() {
                    let k = self.at[v][i];
                    let o = self.other(k, v);
                    if !inside[o] && self.slack(k) == 0 {
                        inside[o] = true;
                        self.tree[k] = true;
                        count += 1;
                        todo.push(o);
                    }
                }
            }
            if count == self.n {
                return;
            }
            let k = (0..self.g.len()).filter(|&k| inside[self.g[k].tail] != inside[self.g[k].head]).min_by_key(|&k| (self.slack(k), k)).expect("the pieces are joined");
            let delta = if inside[self.g[k].tail] { self.slack(k) } else { -self.slack(k) };
            for v in 0..self.n {
                if inside[v] {
                    self.rank[v] += delta;
                }
            }
        }
    }

    /// Parents, postorder numbers and cut values of the tree, from node 0.
    fn number(&mut self) {
        let mut adj = vec![Vec::new(); self.n];
        for (k, e) in self.g.iter().enumerate() {
            if self.tree[k] {
                adj[e.tail].push(k);
                adj[e.head].push(k);
            }
        }
        self.parent.iter_mut().for_each(|p| *p = usize::MAX);
        self.up.iter_mut().for_each(|p| *p = usize::MAX);
        self.post.clear();
        let mut next = 1;
        let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
        self.low[0] = next;
        while let Some(&mut (v, ref mut i)) = stack.last_mut() {
            if *i < adj[v].len() {
                let k = adj[v][*i];
                *i += 1;
                if k == self.up[v] {
                    continue;
                }
                let o = self.other(k, v);
                self.parent[o] = v;
                self.up[o] = k;
                self.low[o] = next;
                stack.push((o, 0));
            } else {
                self.lim[v] = next;
                next += 1;
                self.post.push(v);
                stack.pop();
            }
        }
        for i in 0..self.post.len() {
            let v = self.post[i];
            if self.up[v] != usize::MAX {
                self.cut[self.up[v]] = self.cut_value(v);
            }
        }
    }

    /// The cut value of the tree edge from `child` to its parent, from those below it (dagre's
    /// calcCutValue).
    fn cut_value(&self, child: usize) -> i64 {
        let parent = self.parent[child];
        let up = self.up[child];
        let child_is_tail = self.g[up].tail == child;
        let mut cut = self.g[up].weight;
        for &k in &self.at[child] {
            if k == up {
                continue;
            }
            let e = self.g[k];
            let out = e.tail == child;
            let other = if out { e.head } else { e.tail };
            if other == parent {
                continue;
            }
            let toward_head = out == child_is_tail;
            cut += if toward_head { e.weight } else { -e.weight };
            if self.tree[k] {
                cut += if toward_head { -self.cut[k] } else { self.cut[k] };
            }
        }
        cut
    }

    /// Values from the root down the tree, every tree edge tight.
    fn update_ranks(&mut self) {
        for i in (0..self.post.len()).rev() {
            let v = self.post[i];
            if self.up[v] != usize::MAX {
                let e = self.g[self.up[v]];
                let p = self.parent[v];
                self.rank[v] = if e.tail == p { self.rank[p] + e.minlen } else { self.rank[p] - e.minlen };
            }
        }
    }

    fn run(&mut self) {
        self.number();
        let mut left = EXCHANGES_PER_ITEM * (self.n + self.g.len());
        while left > 0 {
            left -= 1;
            let Some(e) = (0..self.g.len()).find(|&k| self.tree[k] && self.cut[k] < 0) else {
                return;
            };
            let Some(f) = self.enter(e) else {
                return;
            };
            self.tree[e] = false;
            self.tree[f] = true;
            self.number();
            self.update_ranks();
        }
    }

    /// The edge to take in for tree edge `e`: from the head's side of the cut to the tail's, the
    /// least slack.
    fn enter(&self, e: usize) -> Option<usize> {
        let (v, w) = (self.g[e].tail, self.g[e].head);
        // The side under the edge in the tree.
        let (t, flip) = if self.lim[v] > self.lim[w] { (w, true) } else { (v, false) };
        let under = |x: usize| self.low[t] <= self.lim[x] && self.lim[x] <= self.lim[t];
        (0..self.g.len()).filter(|&k| k != e && flip == under(self.g[k].tail) && flip != under(self.g[k].head)).min_by_key(|&k| (self.slack(k), k))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cost(edges: &[Edge], r: &[i64]) -> i64 {
        edges.iter().map(|e| e.weight * (r[e.head] - r[e.tail])).sum()
    }

    fn keeps(edges: &[Edge], r: &[i64]) -> bool {
        edges.iter().all(|e| r[e.head] - r[e.tail] >= e.minlen)
    }

    #[test]
    fn a_node_feeding_far_goes_next_to_what_it_feeds() {
        // 0 → 1 → 2 → 3 → 4, and 5 fed by 0 feeds 4 twice: 5 belongs just before 4.
        let e = |tail, head, weight| Edge { tail, head, minlen: 1, weight };
        let edges = [e(0, 1, 1), e(1, 2, 1), e(2, 3, 1), e(3, 4, 1), e(0, 5, 1), e(5, 4, 2)];
        let r = solve(6, &edges);
        assert!(keeps(&edges, &r));
        assert_eq!(r[5], 3, "{r:?}");
    }

    /// Against every answer, on small random graphs.
    #[test]
    fn the_answer_is_the_best() {
        let mut seed = 12345u64;
        let mut rnd = |m: u64| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) % m
        };
        for _ in 0..200 {
            let n = 5;
            let mut edges = Vec::new();
            for a in 0..n {
                for b in a + 1..n {
                    if rnd(3) == 0 {
                        edges.push(Edge { tail: a, head: b, minlen: rnd(3) as i64, weight: rnd(4) as i64 });
                    }
                }
            }
            let r = solve(n, &edges);
            assert!(keeps(&edges, &r), "{edges:?} {r:?}");
            let mut best = i64::MAX;
            let mut t = vec![0i64; n];
            // Node 0 at 0, the others from −8 to 8 (no tight path is longer than 4 × 2).
            for code in 0..17usize.pow(n as u32 - 1) {
                let mut c = code;
                for v in t.iter_mut().skip(1) {
                    *v = (c % 17) as i64 - 8;
                    c /= 17;
                }
                if keeps(&edges, &t) {
                    best = best.min(cost(&edges, &t));
                }
            }
            assert_eq!(cost(&edges, &r), best, "{edges:?} {r:?}");
        }
    }
}
