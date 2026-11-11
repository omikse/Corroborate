//! Lays nodes out by their wires, left to right: the layered method of Sugiyama, Tagawa and Toda
//! (1981), with the socket heights as they are, which the Rust crates for it do not take. Each step
//! is the one Graphviz dot and ELK (Eclipse Layout Kernel) use:
//!
//! - Columns by network simplex (Gansner et al. 1993): every wire at least one column long and all
//!   of them together as short as they can be (`simplex`).
//! - A wire longer than one column gets a stand-in node in each column it passes, which keeps a
//!   lane for it there, so it runs between nodes and not behind them.
//! - Each column is ordered to reduce crossings: barycentre sweeps on where the wires want each
//!   node, keeping the best order seen, then neighbours swapped while that undoes more (dot's
//!   transpose).
//! - The heights by network simplex on Gansner's auxiliary graph: every wire as level as it can be,
//!   exactly, a long wire's lanes weighing most so that it runs straight.
//!
//! Our own on top: a column far taller than the rest of its piece is dealt out into several side by
//! side, so that fifty recordings stand next to the rows they feed. Separate pieces of the graph
//! are packed in rows the shape of a screen; nodes with no wires are parked in rows underneath.
//! Pure geometry, in canvas units: no state, and the same input gives the same answer.

mod simplex;

/// Space between columns.
const COLUMN_GAP: f32 = 140.0;
/// Space between nodes in a column.
const NODE_GAP: f32 = 40.0;
/// Space between the columns one tall column was dealt into.
const DEALT_GAP: f32 = 40.0;
/// Space between separate pieces of the graph.
const PIECE_GAP: f32 = 120.0;
/// Parked nodes wrap at this width, or at the width of the pieces if that is wider.
const PARK_WIDTH: f32 = 1600.0;
/// The shape aimed for, width over height.
const SCREEN: f32 = 1.8;
/// A column no taller than this is always left whole.
const WRAP_FROM: f32 = 2600.0;
/// Nor one less than this many times the height the piece needs.
const WRAP_OVER: f32 = 1.4;
/// Space between a wire's lane and a node.
const LANE_GAP: f32 = 20.0;
/// Space between two wires' lanes.
const WIRE_GAP: f32 = 10.0;
const ORDER_SWEEPS: usize = 24;
const TRANSPOSE_PASSES: usize = 20;

/// A node as it stands: its size and where it is now.
#[derive(Clone, Copy, Debug)]
pub struct Item {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// A wire. `from_y` and `to_y`: how far below the top of its node each end is.
#[derive(Clone, Copy, Debug)]
pub struct Link {
    pub from: usize,
    pub from_y: f32,
    pub to: usize,
    pub to_y: f32,
}

/// How a layout came out: what the Clean up scenes compare.
#[derive(Clone, Copy, Debug, Default)]
pub struct Report {
    pub width: f32,
    pub height: f32,
    /// Pairs of nodes overlapping.
    pub overlaps: usize,
    /// Pairs of wires crossing, each wire straight from end to end.
    pub crossings: usize,
    /// Wires running behind a node that is not one of their ends.
    pub behind: usize,
    /// Total wire length.
    pub wire: f32,
    /// Wires that run right to left.
    pub backward: usize,
}

/// Measures a layout. `at`: each item's top left.
pub fn measure(items: &[Item], links: &[Link], at: &[(f32, f32)]) -> Report {
    let mut r = Report::default();
    let (mut lo, mut hi) = ((f32::MAX, f32::MAX), (f32::MIN, f32::MIN));
    for (n, i) in items.iter().enumerate() {
        lo = (lo.0.min(at[n].0), lo.1.min(at[n].1));
        hi = (hi.0.max(at[n].0 + i.w), hi.1.max(at[n].1 + i.h));
        for m in n + 1..items.len() {
            if at[n].0 < at[m].0 + items[m].w && at[m].0 < at[n].0 + i.w && at[n].1 < at[m].1 + items[m].h && at[m].1 < at[n].1 + i.h {
                r.overlaps += 1;
            }
        }
    }
    (r.width, r.height) = (hi.0 - lo.0, hi.1 - lo.1);
    let ends: Vec<((f32, f32), (f32, f32))> = links.iter().map(|l| ((at[l.from].0 + items[l.from].w, at[l.from].1 + l.from_y), (at[l.to].0, at[l.to].1 + l.to_y))).collect();
    for (k, l) in links.iter().enumerate() {
        let (a, b) = ends[k];
        r.wire += ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
        r.backward += (b.0 < a.0) as usize;
        for (n, i) in items.iter().enumerate() {
            if n == l.from || n == l.to {
                continue;
            }
            r.behind += (0..=24).any(|s| {
                let t = s as f32 / 24.0;
                let (x, y) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
                x > at[n].0 && x < at[n].0 + i.w && y > at[n].1 && y < at[n].1 + i.h
            }) as usize;
        }
        for &(c, d) in &ends[k + 1..] {
            let side = |p: (f32, f32), q: (f32, f32), s: (f32, f32)| (q.0 - p.0) * (s.1 - p.1) - (q.1 - p.1) * (s.0 - p.0);
            r.crossings += (side(a, b, c) * side(a, b, d) < 0.0 && side(c, d, a) * side(c, d, b) < 0.0) as usize;
        }
    }
    r
}

/// Where each item goes (top left corners), the whole staying where the items' top left was.
pub fn place(items: &[Item], links: &[Link]) -> Vec<(f32, f32)> {
    let mut out = vec![(0.0, 0.0); items.len()];
    if items.is_empty() {
        return out;
    }
    // Whole units: a size off by rounding noise (427.99994) must not tip a tie the other way.
    let items: &[Item] = &items.iter().map(|i| Item { w: i.w.round(), h: i.h.round(), ..*i }).collect::<Vec<_>>();
    let links: Vec<Link> = links.iter().filter(|l| l.from != l.to && l.from < items.len() && l.to < items.len()).map(|l| Link { from_y: l.from_y.round(), to_y: l.to_y.round(), ..*l }).collect();
    let (pieces, parked) = pieces(items, &links);
    let sizes: Vec<(f32, f32)> = pieces.iter().map(|piece| place_piece(piece, items, &links, &mut out)).collect();
    // The pieces in rows, as wide as makes the whole about the shape of a screen.
    let area: f32 = sizes.iter().map(|s| (s.0 + PIECE_GAP) * (s.1 + PIECE_GAP)).sum();
    let widest = sizes.iter().map(|s| s.0).fold(0.0, f32::max);
    let limit = widest.max((area * SCREEN).sqrt());
    let (mut x, mut top, mut row) = (0.0f32, 0.0f32, 0.0f32);
    let mut used = 0.0f32;
    for (piece, size) in pieces.iter().zip(&sizes) {
        if x > 0.0 && x + size.0 > limit {
            top += row + PIECE_GAP;
            (x, row) = (0.0, 0.0);
        }
        for &n in piece {
            out[n] = (out[n].0 + x, out[n].1 + top);
        }
        used = used.max(x + size.0);
        x += size.0 + PIECE_GAP;
        row = row.max(size.1);
    }
    if row > 0.0 {
        top += row + PIECE_GAP;
    }
    // Rows of parked nodes.
    let limit = used.max(PARK_WIDTH);
    let (mut x, mut row) = (0.0f32, 0.0f32);
    for &n in &parked {
        if x > 0.0 && x + items[n].w > limit {
            top += row + NODE_GAP;
            (x, row) = (0.0, 0.0);
        }
        out[n] = (x, top);
        x += items[n].w + NODE_GAP;
        row = row.max(items[n].h);
    }
    let left = items.iter().map(|i| i.x).fold(f32::INFINITY, f32::min);
    let up = items.iter().map(|i| i.y).fold(f32::INFINITY, f32::min);
    for p in &mut out {
        *p = (p.0 + left, p.1 + up);
    }
    out
}

/// Top to bottom as they stand now.
fn by_place(items: &[Item], a: usize, b: usize) -> std::cmp::Ordering {
    items[a].y.total_cmp(&items[b].y).then(items[a].x.total_cmp(&items[b].x)).then(a.cmp(&b))
}

/// The connected pieces of the graph, and the nodes with no wires, each top to bottom as they
/// stand now.
fn pieces(items: &[Item], links: &[Link]) -> (Vec<Vec<usize>>, Vec<usize>) {
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    let mut p: Vec<usize> = (0..items.len()).collect();
    let mut wired = vec![false; items.len()];
    for l in links {
        let (a, b) = (find(&mut p, l.from), find(&mut p, l.to));
        p[a] = b;
        wired[l.from] = true;
        wired[l.to] = true;
    }
    let mut groups: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    let mut parked = Vec::new();
    for n in 0..items.len() {
        if wired[n] {
            groups.entry(find(&mut p, n)).or_default().push(n);
        } else {
            parked.push(n);
        }
    }
    parked.sort_by(|&a, &b| by_place(items, a, b));
    let mut pieces: Vec<Vec<usize>> = groups.into_values().collect();
    for piece in &mut pieces {
        piece.sort_by(|&a, &b| by_place(items, a, b));
    }
    pieces.sort_by(|a, b| by_place(items, a[0], b[0]));
    (pieces, parked)
}

/// A wire of a piece, left to right: (left node, its socket's height, right node, its socket's
/// height).
type Wire = (usize, f32, usize, f32);

/// The piece's wires, each pointing right. A loop can only appear between groups, at the level
/// they are shown: it is broken at the first node waiting, and the wire closing it is turned round.
fn forward(piece: &[usize], links: &[Link], count: usize) -> Vec<Wire> {
    let mut inside = vec![false; count];
    for &n in piece {
        inside[n] = true;
    }
    let links: Vec<&Link> = links.iter().filter(|l| inside[l.from]).collect();
    let mut waiting = vec![0usize; count];
    for l in &links {
        waiting[l.to] += 1;
    }
    let mut seq = vec![0usize; count];
    let mut done = vec![false; count];
    let mut left: Vec<usize> = piece.to_vec();
    for k in 0..piece.len() {
        let i = left.iter().position(|&n| waiting[n] == 0).unwrap_or(0);
        let n = left.remove(i);
        done[n] = true;
        seq[n] = k;
        for l in links.iter().filter(|l| l.from == n && !done[l.to]) {
            waiting[l.to] = waiting[l.to].saturating_sub(1);
        }
    }
    links.iter().map(|l| if seq[l.from] < seq[l.to] { (l.from, l.from_y, l.to, l.to_y) } else { (l.to, l.to_y, l.from, l.from_y) }).collect()
}

/// Which column each node of a piece is in: every wire at least one column long and all of them
/// together as short as they can be (network simplex, as Graphviz dot ranks). A node with no
/// inputs so stands just before what it feeds, and a node at the end of a branch just after what
/// feeds it.
fn columns(piece: &[usize], wires: &[Wire], count: usize) -> Vec<usize> {
    let mut slot = vec![usize::MAX; count];
    for (k, &n) in piece.iter().enumerate() {
        slot[n] = k;
    }
    let edges: Vec<simplex::Edge> = wires.iter().map(|w| simplex::Edge { tail: slot[w.0], head: slot[w.2], minlen: 1, weight: 1 }).collect();
    let r = simplex::solve(piece.len(), &edges);
    let mut column = vec![0usize; count];
    for (k, &n) in piece.iter().enumerate() {
        column[n] = r[k] as usize;
    }
    column
}

/// Deals a column taller than `limit` into several, left to right. A node whose wires all come
/// from the left (a Map at the end of a branch) goes in the leftmost that has room, next to what
/// feeds it; the others each go to the shortest so far. The order down each one stays the column's.
fn deal(col: &[usize], items: &[Item], limit: f32, feeds_on: &[bool]) -> Vec<Vec<usize>> {
    let height: f32 = col.iter().map(|&n| items[n].h + NODE_GAP).sum();
    if height <= (limit * WRAP_OVER).max(WRAP_FROM) {
        return vec![col.to_vec()];
    }
    let k = (height / limit).ceil() as usize;
    let mut which = vec![0usize; items.len()];
    let mut used = vec![0.0f32; k];
    for &n in col.iter().filter(|&&n| !feeds_on[n]) {
        let s = (0..k).find(|&s| used[s] + items[n].h <= limit).unwrap_or(k - 1);
        which[n] = s;
        used[s] += items[n].h + NODE_GAP;
    }
    for &n in col.iter().filter(|&&n| feeds_on[n]) {
        let s = (0..k).min_by(|&a, &b| used[a].total_cmp(&used[b])).unwrap_or(0);
        which[n] = s;
        used[s] += items[n].h + NODE_GAP;
    }
    let mut out = vec![Vec::new(); k];
    for &n in col {
        out[which[n]].push(n);
    }
    out
}

/// Lays out one piece from (0, 0); returns its width and height.
fn place_piece(piece: &[usize], items: &[Item], links: &[Link], out: &mut [(f32, f32)]) -> (f32, f32) {
    // By id, not by where they stand: the layout depends on the graph alone, so a second press
    // changes nothing.
    let mut piece = piece.to_vec();
    piece.sort_unstable();
    let piece = &piece[..];
    let wires = forward(piece, links, items.len());
    let column = columns(piece, &wires, items.len());
    let mut lay = Layered::build(piece, &column, &wires, items);
    lay.order();

    // Columns much taller than the piece needs are dealt out into several side by side. The lanes
    // of wires passing such a column keep a column of their own: the room is between its nodes'
    // wires, not between the nodes, so a lane there would only push them off the rows they feed.
    let area: f32 = piece.iter().map(|&n| (items[n].w + COLUMN_GAP) * (items[n].h + NODE_GAP)).sum();
    let tallest = piece.iter().map(|&n| items[n].h).fold(0.0, f32::max);
    let limit = tallest.max((area / SCREEN).sqrt());
    let mut feeds_on = vec![false; items.len()];
    for w in &wires {
        feeds_on[w.0] = true;
    }
    // The gap after each column.
    let mut gaps = Vec::new();
    let mut split = Vec::new();
    for layer in &lay.layers {
        let col: Vec<usize> = layer.iter().filter_map(|&v| lay.vn[v].item).collect();
        let dealt = deal(&col, items, limit, &feeds_on);
        if dealt.len() == 1 {
            split.push(layer.clone());
            gaps.push(COLUMN_GAP);
            continue;
        }
        let mut slot = vec![usize::MAX; items.len()];
        for &v in layer {
            if let Some(n) = lay.vn[v].item {
                slot[n] = v;
            }
        }
        let last = dealt.len() - 1;
        for (k, c) in dealt.into_iter().enumerate() {
            split.push(c.into_iter().map(|n| slot[n]).collect());
            gaps.push(if k == last { COLUMN_GAP } else { DEALT_GAP });
        }
        split.push(layer.iter().copied().filter(|&v| lay.vn[v].item.is_none()).collect());
        gaps.push(0.0);
    }
    lay.layers = split;

    let y = lay.heights();
    let real = || lay.vn.iter().enumerate().filter_map(|(v, n)| n.item.map(|i| (v, i)));
    let up = real().map(|(v, _)| y[v]).fold(f32::INFINITY, f32::min);
    let down = real().map(|(v, n)| y[v] + items[n].h).fold(f32::NEG_INFINITY, f32::max);
    let (mut x, mut right) = (0.0f32, 0.0f32);
    for (layer, gap) in lay.layers.iter().zip(&gaps) {
        let nodes: Vec<(usize, usize)> = layer.iter().filter_map(|&v| lay.vn[v].item.map(|n| (v, n))).collect();
        // A column only wires pass through takes no room.
        if nodes.is_empty() {
            continue;
        }
        for &(v, n) in &nodes {
            out[n] = (x, y[v] - up);
        }
        let w = nodes.iter().map(|&(_, n)| items[n].w).fold(0.0, f32::max);
        right = x + w;
        x += w + gap;
    }
    (right, down - up)
}

/// A node of the layered graph: a real node, or a stand-in where a long wire passes a column
/// (Sugiyama's dummy node), keeping a lane for it there. The wires of one pair of nodes share one
/// lane, as tall as their sockets are spread.
#[derive(Clone, Copy, Debug)]
struct Vn {
    item: Option<usize>,
    h: f32,
}

/// A wire from one column to the next: from `a` at `a_y` below its top to `b` at `b_y`; `w`: how
/// many wires it stands for.
#[derive(Clone, Copy, Debug)]
struct Seg {
    a: usize,
    a_y: f32,
    b: usize,
    b_y: f32,
    w: u32,
}

/// A piece as columns of nodes and lanes, every wire from one column to the next.
struct Layered {
    vn: Vec<Vn>,
    seg: Vec<Seg>,
    /// Each column, top to bottom.
    layers: Vec<Vec<usize>>,
    /// The segments at each node.
    touching: Vec<Vec<usize>>,
}

impl Layered {
    /// `nodes` in the order they start in; `column`: each node's column.
    fn build(nodes: &[usize], column: &[usize], wires: &[Wire], items: &[Item]) -> Self {
        let mut vn = Vec::new();
        let mut slot = vec![usize::MAX; items.len()];
        let count = nodes.iter().map(|&n| column[n] + 1).max().unwrap_or(0);
        let mut layers = vec![Vec::new(); count];
        for &n in nodes {
            slot[n] = vn.len();
            layers[column[n]].push(vn.len());
            vn.push(Vn { item: Some(n), h: items[n].h });
        }
        let mut pairs: std::collections::BTreeMap<(usize, usize), Vec<(f32, f32)>> = Default::default();
        for &(a, a_y, b, b_y) in wires {
            pairs.entry((a, b)).or_default().push((a_y, b_y));
        }
        let mut seg = Vec::new();
        for ((a, b), ends) in pairs {
            let (from, to) = (column[a], column[b]);
            if to <= from + 1 {
                seg.extend(ends.iter().map(|&(a_y, b_y)| Seg { a: slot[a], a_y, b: slot[b], b_y, w: 1 }));
                continue;
            }
            let lo = ends.iter().map(|e| e.0).fold(f32::INFINITY, f32::min);
            let hi = ends.iter().map(|e| e.0).fold(f32::NEG_INFINITY, f32::max);
            let lane: Vec<usize> = (from + 1..to)
                .map(|c| {
                    layers[c].push(vn.len());
                    vn.push(Vn { item: None, h: hi - lo });
                    vn.len() - 1
                })
                .collect();
            for &(a_y, b_y) in &ends {
                seg.push(Seg { a: slot[a], a_y, b: lane[0], b_y: a_y - lo, w: 1 });
                seg.push(Seg { a: lane[lane.len() - 1], a_y: a_y - lo, b: slot[b], b_y, w: 1 });
            }
            for p in lane.windows(2) {
                seg.push(Seg { a: p[0], a_y: 0.0, b: p[1], b_y: 0.0, w: ends.len() as u32 });
            }
        }
        let mut touching = vec![Vec::new(); vn.len()];
        for (k, s) in seg.iter().enumerate() {
            touching[s.a].push(k);
            touching[s.b].push(k);
        }
        Layered { vn, seg, layers, touching }
    }

    /// The least space between two neighbours in a column, the upper one first.
    fn sep(&self, p: usize, q: usize) -> f32 {
        match (self.vn[p].item.is_some(), self.vn[q].item.is_some()) {
            (true, true) => NODE_GAP,
            (false, false) => WIRE_GAP,
            _ => LANE_GAP,
        }
    }

    /// Every column stacked from the top.
    fn stack(&self, y: &mut [f32]) {
        for col in &self.layers {
            let mut at = 0.0;
            for (k, &v) in col.iter().enumerate() {
                y[v] = at;
                at += self.vn[v].h + col.get(k + 1).map_or(0.0, |&q| self.sep(v, q));
            }
        }
    }

    /// Where node `v`'s top would make its wires level, on average, and how many wires say so. The
    /// mean, not the median: a recording and the Time node it feeds would otherwise agree with each
    /// other and drift away from the row they both lead to.
    fn wanted(&self, v: usize, y: &[f32]) -> (f32, f32) {
        let (mut sum, mut count) = (0.0, 0.0);
        for &k in &self.touching[v] {
            let s = &self.seg[k];
            let w = s.w as f32;
            sum += w * if s.a == v { y[s.b] + s.b_y - s.a_y } else { y[s.a] + s.a_y - s.b_y };
            count += w;
        }
        if count == 0.0 {
            (y[v], 1.0)
        } else {
            (sum / count, count)
        }
    }

    /// Moves column `c`'s nodes as near as they can get to where their wires want them, keeping
    /// the order and the gaps: weighted isotonic regression by pooling adjacent violators.
    fn settle(&self, c: usize, y: &mut [f32]) {
        let col = &self.layers[c];
        // Each pool is (sum of weight × target, sum of weight, nodes in it).
        let mut pools: Vec<(f32, f32, usize)> = Vec::with_capacity(col.len());
        let mut offset = Vec::with_capacity(col.len());
        let mut stacked = 0.0;
        for (k, &v) in col.iter().enumerate() {
            let (want, weight) = self.wanted(v, y);
            offset.push(stacked);
            pools.push((weight * (want - stacked), weight, 1));
            stacked += self.vn[v].h + col.get(k + 1).map_or(0.0, |&q| self.sep(v, q));
            while pools.len() > 1 {
                let (a, b) = (pools[pools.len() - 2], pools[pools.len() - 1]);
                if a.0 / a.1 <= b.0 / b.1 {
                    break;
                }
                pools.pop();
                *pools.last_mut().unwrap() = (a.0 + b.0, a.1 + b.1, a.2 + b.2);
            }
        }
        let mut k = 0;
        for (sum, weight, nodes) in pools {
            for _ in 0..nodes {
                y[col[k]] = sum / weight + offset[k];
                k += 1;
            }
        }
    }

    /// Sweeps the columns one way and the other, each node going where its wires want it.
    fn relax(&self, y: &mut [f32], sweeps: usize) {
        let k = self.layers.len();
        for sweep in 0..sweeps {
            for c in 0..k {
                self.settle(if sweep % 2 == 0 { c } else { k - 1 - c }, y);
            }
        }
    }

    /// Each node's place in its column.
    fn places(&self) -> Vec<usize> {
        let mut at = vec![0; self.vn.len()];
        for col in &self.layers {
            for (k, &v) in col.iter().enumerate() {
                at[v] = k;
            }
        }
        at
    }

    /// Pairs of wires crossing, each weighted by how many wires it stands for. Between two
    /// neighbouring columns two wires cross when their ends come in the opposite order on each
    /// side, a node's sockets in the order they are down it.
    fn crossings(&self) -> u64 {
        let at = self.places();
        let mut column = vec![0; self.vn.len()];
        for (c, col) in self.layers.iter().enumerate() {
            for &v in col {
                column[v] = c;
            }
        }
        let mut between = vec![Vec::new(); self.layers.len()];
        for s in &self.seg {
            between[column[s.a]].push(((at[s.a], s.a_y), (at[s.b], s.b_y), s.w as u64));
        }
        let mut n = 0;
        for list in &between {
            for (k, p) in list.iter().enumerate() {
                for q in &list[k + 1..] {
                    if (before(p.0, q.0) && before(q.1, p.1)) || (before(q.0, p.0) && before(p.1, q.1)) {
                        n += p.2 * q.2;
                    }
                }
            }
        }
        n
    }

    /// Crossings between `u`'s wires and `v`'s with `u` just above `v` in their column.
    fn pair_crossings(&self, u: usize, v: usize, at: &[usize]) -> u64 {
        let end = |k: usize, me: usize| {
            let s = &self.seg[k];
            if s.a == me {
                (false, (at[s.b], s.b_y), s.w as u64)
            } else {
                (true, (at[s.a], s.a_y), s.w as u64)
            }
        };
        let mut n = 0;
        for &e in &self.touching[u] {
            let (left_e, ke, we) = end(e, u);
            for &f in &self.touching[v] {
                let (left_f, kf, wf) = end(f, v);
                if left_e == left_f && before(kf, ke) {
                    n += we * wf;
                }
            }
        }
        n
    }

    /// Swaps neighbours in a column wherever that leaves fewer crossings (Graphviz dot's transpose
    /// step), until no swap helps.
    fn transpose(&mut self) {
        let mut at = self.places();
        for _ in 0..TRANSPOSE_PASSES {
            let mut swapped = false;
            for c in 0..self.layers.len() {
                for i in 0..self.layers[c].len().saturating_sub(1) {
                    let (u, v) = (self.layers[c][i], self.layers[c][i + 1]);
                    if self.pair_crossings(v, u, &at) < self.pair_crossings(u, v, &at) {
                        self.layers[c].swap(i, i + 1);
                        (at[u], at[v]) = (i + 1, i);
                        swapped = true;
                    }
                }
            }
            if !swapped {
                break;
            }
        }
    }

    /// The order of each column: sweep both ways, each node to where its wires want it, keep the
    /// order with the fewest crossings, then swap neighbours while that undoes more.
    fn order(&mut self) {
        let mut y = vec![0.0f32; self.vn.len()];
        self.stack(&mut y);
        self.relax(&mut y, ORDER_SWEEPS);
        let mut best = self.layers.clone();
        let mut fewest = self.crossings();
        for sweep in 0..ORDER_SWEEPS {
            let k = self.layers.len();
            for c in 0..k {
                let c = if sweep % 2 == 0 { c } else { k - 1 - c };
                let mut keyed: Vec<(f32, usize)> = self.layers[c].iter().map(|&v| (self.wanted(v, &y).0 + self.vn[v].h / 2.0, v)).collect();
                keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
                self.layers[c] = keyed.into_iter().map(|(_, v)| v).collect();
                self.settle(c, &mut y);
            }
            let now = self.crossings();
            if now < fewest {
                fewest = now;
                best = self.layers.clone();
            }
        }
        self.layers = best;
        self.transpose();
    }

    /// Each node's top, the order of every column kept: the wires as level as they can be, a
    /// long wire's lanes weighing most so that it runs straight (Gansner's priority 1, 2, 8), by
    /// network simplex on Gansner's auxiliary graph. Exact, with every socket where it is.
    fn heights(&self) -> Vec<f32> {
        let nv = self.vn.len();
        let mut edges = Vec::new();
        for col in &self.layers {
            for p in col.windows(2) {
                edges.push(simplex::Edge { tail: p[0], head: p[1], minlen: (self.vn[p[0]].h + self.sep(p[0], p[1])).round() as i64, weight: 0 });
            }
        }
        // Each wire becomes a node below both its ends, as far below them as the wire is out of
        // level: pulling it up to both levels the wire.
        for (k, s) in self.seg.iter().enumerate() {
            let lanes = self.vn[s.a].item.is_none() as usize + self.vn[s.b].item.is_none() as usize;
            let weight = [1, 2, 8][lanes] * s.w as i64;
            let c = s.a_y.max(s.b_y);
            edges.push(simplex::Edge { tail: nv + k, head: s.a, minlen: (c - s.a_y).round() as i64, weight });
            edges.push(simplex::Edge { tail: nv + k, head: s.b, minlen: (c - s.b_y).round() as i64, weight });
        }
        simplex::solve(nv + self.seg.len(), &edges)[..nv].iter().map(|&r| r as f32).collect()
    }
}

/// Higher up: an earlier node in the column, or the same node and a higher socket.
fn before(p: (usize, f32), q: (usize, f32)) -> bool {
    p.0 < q.0 || (p.0 == q.0 && p.1 < q.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(y: f32, h: f32) -> Item {
        Item { x: 0.0, y, w: 200.0, h }
    }

    fn link(from: usize, to: usize, from_y: f32, to_y: f32) -> Link {
        Link { from, from_y, to, to_y }
    }

    /// True when no two boxes overlap.
    fn apart(items: &[Item], at: &[(f32, f32)]) -> bool {
        for a in 0..items.len() {
            for b in a + 1..items.len() {
                let x = at[a].0 < at[b].0 + items[b].w && at[b].0 < at[a].0 + items[a].w;
                let y = at[a].1 < at[b].1 + items[b].h && at[b].1 < at[a].1 + items[a].h;
                if x && y {
                    return false;
                }
            }
        }
        true
    }

    #[test]
    fn a_chain_runs_left_to_right_with_level_wires() {
        let items = [item(500.0, 100.0), item(0.0, 300.0), item(40.0, 80.0)];
        let links = [link(0, 1, 50.0, 200.0), link(1, 2, 70.0, 10.0)];
        let at = place(&items, &links);
        assert!(at[0].0 < at[1].0 && at[1].0 < at[2].0, "{at:?}");
        assert!((at[0].1 + 50.0 - (at[1].1 + 200.0)).abs() < 0.5, "{at:?}");
        assert!((at[1].1 + 70.0 - (at[2].1 + 10.0)).abs() < 0.5, "{at:?}");
    }

    /// Recordings wired to the rows of one tall node come out in the order of the rows, whatever
    /// order they stood in.
    #[test]
    fn sources_line_up_with_the_rows_they_feed() {
        let mut items: Vec<Item> = (0..6).map(|k| item(k as f32 * 150.0, 120.0)).collect();
        items.push(item(0.0, 700.0));
        // Source k feeds the row (5 - k), with two wires like an audio and a time.
        let links: Vec<Link> = (0..6).flat_map(|k| [link(k, 6, 30.0, 50.0 + (5 - k) as f32 * 100.0), link(k, 6, 50.0, 70.0 + (5 - k) as f32 * 100.0)]).collect();
        let at = place(&items, &links);
        assert!(apart(&items, &at), "{at:?}");
        for k in 0..5 {
            assert!(at[k].1 > at[k + 1].1, "row order: {at:?}");
            assert_eq!(at[k].0, at[0].0, "one column");
        }
        assert!(at[6].0 > at[0].0 + 200.0);
    }

    #[test]
    fn crossings_come_undone() {
        // a above b feed d above c: as they stand, the two wires cross.
        let items = [item(0.0, 100.0), item(200.0, 100.0), item(0.0, 100.0), item(200.0, 100.0)];
        let links = [link(0, 3, 50.0, 50.0), link(1, 2, 50.0, 50.0)];
        // Separate pieces are stacked: join them through a fifth node so they share columns.
        let mut joined = items.to_vec();
        joined.push(item(100.0, 100.0));
        let mut wires = links.to_vec();
        wires.push(link(2, 4, 50.0, 25.0));
        wires.push(link(3, 4, 50.0, 75.0));
        let at = place(&joined, &wires);
        assert!(apart(&joined, &at), "{at:?}");
        let a = (at[0].1 - at[1].1).signum();
        let b = (at[3].1 - at[2].1).signum();
        assert_eq!(a, b, "the wires no longer cross: {at:?}");
    }

    #[test]
    fn a_source_stands_next_to_what_it_feeds() {
        let items = [item(0.0, 80.0), item(0.0, 80.0), item(0.0, 80.0), item(200.0, 80.0)];
        // 0 → 1 → 2, and 3 feeds only 2.
        let at = place(&items, &[link(0, 1, 40.0, 40.0), link(1, 2, 40.0, 40.0), link(3, 2, 40.0, 60.0)]);
        assert_eq!(at[3].0, at[1].0, "{at:?}");
    }

    #[test]
    fn pieces_stack_and_unwired_nodes_are_parked_underneath() {
        let items = [item(900.0, 100.0), item(900.0, 100.0), item(0.0, 100.0), item(0.0, 100.0), item(50.0, 60.0), item(70.0, 60.0)];
        let at = place(&items, &[link(0, 1, 50.0, 50.0), link(2, 3, 50.0, 50.0)]);
        assert!(apart(&items, &at), "{at:?}");
        assert!(at[2].1 < at[0].1, "the piece higher up stays higher: {at:?}");
        assert!(at[4].1 > at[0].1 + 100.0 && at[5].1 == at[4].1, "parked in a row under them: {at:?}");
    }

    #[test]
    fn stays_in_the_same_corner() {
        let items = [Item { x: 300.0, y: 700.0, w: 100.0, h: 100.0 }, Item { x: 900.0, y: 400.0, w: 100.0, h: 100.0 }];
        let at = place(&items, &[link(0, 1, 50.0, 50.0)]);
        assert_eq!(at[0], (300.0, 400.0));
    }

    #[test]
    fn a_loop_does_not_hang_and_the_same_input_gives_the_same_answer() {
        let items = [item(0.0, 100.0), item(150.0, 100.0), item(300.0, 100.0)];
        let links = [link(0, 1, 50.0, 50.0), link(1, 2, 50.0, 50.0), link(2, 0, 50.0, 50.0), link(1, 1, 10.0, 10.0)];
        let a = place(&items, &links);
        assert!(apart(&items, &a), "{a:?}");
        assert_eq!(a, place(&items, &links));
        assert!(place(&[], &[]).is_empty());
    }

    /// A column far taller than what it feeds is dealt into several, in the order of the rows, and
    /// each node stands level with its row.
    #[test]
    fn a_very_tall_column_is_dealt_beside_what_it_feeds() {
        let mut items: Vec<Item> = (0..40).map(|k| item(k as f32 * 7.0, 170.0)).collect();
        items.push(Item { x: 0.0, y: 0.0, w: 980.0, h: 2600.0 });
        let links: Vec<Link> = (0..40).map(|k| link(k, 40, 60.0, 30.0 + k as f32 * 64.0)).collect();
        let at = place(&items, &links);
        assert!(apart(&items, &at), "{at:?}");
        let mut lefts: Vec<i32> = at[..40].iter().map(|p| p.0 as i32).collect();
        lefts.sort_unstable();
        lefts.dedup();
        assert!(lefts.len() >= 3, "{lefts:?}");
        let r = measure(&items, &links, &at);
        assert!(r.height <= 3000.0 && r.backward == 0, "{r:?}");
        for k in 0..40 {
            assert!((at[k].1 + 60.0 - (at[40].1 + 30.0 + k as f32 * 64.0)).abs() < 200.0, "node {k} is near its row: {at:?}");
        }
    }

    /// A node at the end of a branch stays beside what feeds it when its column is dealt out,
    /// with nothing between them: the recordings that go on further right take the other columns.
    #[test]
    fn an_end_node_stays_next_to_what_feeds_it() {
        // 0 feeds 1 and also 2, 2 is one of many feeding the tall node 3 at the far right; 1 ends.
        let mut items = vec![item(0.0, 170.0), item(9.0, 400.0)];
        let mut links = vec![link(0, 1, 60.0, 30.0)];
        items.push(item(20.0, 170.0));
        links.push(link(0, 2, 60.0, 30.0));
        let sink = 40;
        for k in 3..sink {
            items.push(item(k as f32 * 8.0, 170.0));
        }
        items.push(Item { x: 0.0, y: 0.0, w: 980.0, h: 2600.0 });
        for k in 2..sink {
            links.push(link(k, sink, 60.0, 30.0 + k as f32 * 64.0));
        }
        let at = place(&items, &links);
        assert!(apart(&items, &at), "{at:?}");
        let gap = at[1].0 - (at[0].0 + items[0].w);
        assert!((gap - COLUMN_GAP).abs() < 1.0, "the end node is in the first column after its feeder: gap {gap}");
    }

    /// A wire skipping columns gets a lane through them: it runs level and behind no node.
    #[test]
    fn a_long_wire_runs_in_a_lane() {
        // 0 → 1 → 2 → 3 → 4, and 0 straight to 4; 5 and 6 hang off 2 so that its column is full.
        let items: Vec<Item> = (0..7).map(|k| item(k as f32 * 50.0, 120.0)).collect();
        let links = [link(0, 1, 60.0, 60.0), link(1, 2, 60.0, 60.0), link(2, 3, 60.0, 60.0), link(3, 4, 60.0, 30.0), link(0, 4, 90.0, 90.0), link(1, 5, 90.0, 60.0), link(1, 6, 30.0, 60.0)];
        let at = place(&items, &links);
        let r = measure(&items, &links, &at);
        assert!(r.behind == 0 && r.overlaps == 0, "{r:?} {at:?}");
        assert!((at[0].1 + 90.0 - (at[4].1 + 90.0)).abs() < 0.5, "the long wire is level: {at:?}");
    }

    /// A node fed early and feeding late stands next to what it feeds: two wires kept short
    /// rather than one.
    #[test]
    fn columns_keep_the_wires_short() {
        let items: Vec<Item> = (0..6).map(|k| item(k as f32 * 150.0, 100.0)).collect();
        let links = [link(0, 1, 50.0, 50.0), link(1, 2, 50.0, 50.0), link(2, 3, 50.0, 50.0), link(3, 4, 50.0, 30.0), link(0, 5, 70.0, 50.0), link(5, 4, 30.0, 60.0), link(5, 4, 60.0, 80.0)];
        let at = place(&items, &links);
        assert_eq!(at[5].0, at[3].0, "{at:?}");
    }

    /// An ordinary column is left whole, even when it is a few nodes tall.
    #[test]
    fn a_small_column_is_left_whole() {
        let mut items: Vec<Item> = (0..8).map(|k| item(k as f32, 230.0)).collect();
        items.push(item(0.0, 400.0));
        let links: Vec<Link> = (0..8).map(|k| link(k, 8, 60.0, 30.0 + k as f32 * 40.0)).collect();
        let at = place(&items, &links);
        assert!(at[..8].iter().all(|p| p.0 == at[0].0), "{at:?}");
    }

    /// Where the nodes stood does not matter: the same graph comes out the same.
    #[test]
    fn the_layout_depends_on_the_graph_alone() {
        let build = |shift: f32| -> Vec<Item> { (0..7).map(|k| Item { x: 0.0, y: ((k as f32 + 1.0) * 977.0 * shift) % 1500.0, w: 200.0, h: 120.0 }).collect() };
        let links = [link(0, 3, 30.0, 40.0), link(1, 3, 30.0, 80.0), link(2, 4, 30.0, 40.0), link(3, 5, 60.0, 30.0), link(4, 5, 60.0, 90.0), link(5, 6, 60.0, 60.0)];
        let (a, b) = (place(&build(1.0), &links), place(&build(3.7), &links));
        let origin = |p: &[(f32, f32)]| p.iter().fold((f32::MAX, f32::MAX), |m, q| (m.0.min(q.0), m.1.min(q.1)));
        let (oa, ob) = (origin(&a), origin(&b));
        for (p, q) in a.iter().zip(&b) {
            assert!((p.0 - oa.0 - q.0 + ob.0).abs() < 0.01 && (p.1 - oa.1 - q.1 + ob.1).abs() < 0.01, "{a:?}\n{b:?}");
        }
    }

    /// Small separate pieces share a row instead of making one long stack.
    #[test]
    fn small_pieces_share_a_row() {
        let items: Vec<Item> = (0..12).map(|k| item(k as f32 * 200.0, 100.0)).collect();
        let links: Vec<Link> = (0..6).map(|k| link(2 * k, 2 * k + 1, 50.0, 50.0)).collect();
        let at = place(&items, &links);
        assert!(apart(&items, &at), "{at:?}");
        let r = measure(&items, &links, &at);
        assert!(r.width / r.height > 1.0, "{r:?}");
    }

    /// Tidying what is already tidy changes nothing more.
    #[test]
    fn a_second_cleanup_changes_nothing() {
        let items: Vec<Item> = (0..5).map(|k| item((4 - k) as f32 * 130.0, 110.0)).chain([item(0.0, 500.0), item(100.0, 90.0)]).collect();
        let mut links: Vec<Link> = (0..5).map(|k| link(k, 5, 30.0, 40.0 + k as f32 * 90.0)).collect();
        links.push(link(5, 6, 250.0, 40.0));
        let a = place(&items, &links);
        let moved: Vec<Item> = items.iter().zip(&a).map(|(i, p)| Item { x: p.0, y: p.1, ..*i }).collect();
        let b = place(&moved, &links);
        for (p, q) in a.iter().zip(&b) {
            assert!((p.0 - q.0).abs() < 0.5 && (p.1 - q.1).abs() < 1.0, "{a:?}\n{b:?}");
        }
    }
}
