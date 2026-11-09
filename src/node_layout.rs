//! Lays nodes out by their wires, left to right: the layered method of Sugiyama, Tagawa and Toda
//! (1981), with the port heights as they are, which the Rust crates for it do not take.
//!
//! A node's column is how many steps it is from the start of the graph; a node with no inputs
//! stands just before what it feeds. Each column is ordered to reduce crossings (barycentre sweeps,
//! keeping the best seen), then each node slides to where its wires run straightest, without
//! overlapping its neighbours or changing the order (pool adjacent violators).
//!
//! A column far taller than the rest of its piece is dealt out into several side by side, so that
//! fifty recordings stand next to the rows they feed. Separate pieces of the graph are packed in
//! rows the shape of a screen; nodes with no wires are parked in rows underneath.
//! Pure geometry, in canvas units: no state, and the same input gives the same answer.

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
const ORDER_SWEEPS: usize = 24;
const PLACE_SWEEPS: usize = 40;

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

/// Which column each node of a piece is in. A loop can only appear between groups, at the level
/// they are shown: it is broken at the topmost node waiting.
fn columns(piece: &[usize], links: &[Link], count: usize) -> Vec<usize> {
    let mut column = vec![0usize; count];
    let mut waiting = vec![0usize; count];
    let mut done = vec![false; count];
    for l in links {
        waiting[l.to] += 1;
    }
    let mut left: Vec<usize> = piece.to_vec();
    let mut fed = vec![false; count];
    while !left.is_empty() {
        let k = left.iter().position(|&n| waiting[n] == 0).unwrap_or(0);
        let n = left.remove(k);
        done[n] = true;
        for l in links.iter().filter(|l| l.from == n && !done[l.to]) {
            column[l.to] = column[l.to].max(column[n] + 1);
            waiting[l.to] = waiting[l.to].saturating_sub(1);
            fed[l.to] = true;
        }
    }
    // A node with no inputs stands just before the first thing it feeds, not at the far left.
    for &n in piece {
        if !fed[n] {
            if let Some(first) = links.iter().filter(|l| l.from == n).map(|l| column[l.to]).min() {
                column[n] = first.saturating_sub(1);
            }
        }
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
    let column = columns(piece, links, items.len());
    let last = piece.iter().map(|&n| column[n]).max().unwrap_or(0);
    let mut cols: Vec<Vec<usize>> = vec![Vec::new(); last + 1];
    for &n in piece {
        cols[column[n]].push(n);
    }
    let mut touching: Vec<Vec<usize>> = vec![Vec::new(); items.len()];
    for (k, l) in links.iter().enumerate() {
        touching[l.from].push(k);
        touching[l.to].push(k);
    }
    let mut y = vec![0.0f32; items.len()];
    stack(&cols, items, &mut y);
    relax(&cols, items, links, &touching, &mut y, ORDER_SWEEPS);

    // The order of each column: sweep both ways and keep the fewest crossings. Measured settled,
    // so that a layout already at its best is not traded for an equal one.
    let mut best = cols.clone();
    let mut fewest = crossings(links, &column, &y);
    for sweep in 0..ORDER_SWEEPS {
        let k = cols.len();
        for c in 0..k {
            let col = &mut cols[if sweep % 2 == 0 { c } else { k - 1 - c }];
            let mut keyed: Vec<(f32, usize)> = col.iter().map(|&n| (wanted(n, links, &touching, &y).0 + items[n].h / 2.0, n)).collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
            *col = keyed.into_iter().map(|(_, n)| n).collect();
            settle(col, items, links, &touching, &mut y);
        }
        let now = crossings(links, &column, &y);
        if now < fewest {
            fewest = now;
            best = cols.clone();
        }
    }

    // Columns much taller than the piece needs are dealt out, then the heights are found.
    let area: f32 = piece.iter().map(|&n| (items[n].w + COLUMN_GAP) * (items[n].h + NODE_GAP)).sum();
    let tallest = piece.iter().map(|&n| items[n].h).fold(0.0, f32::max);
    let limit = tallest.max((area / SCREEN).sqrt());
    let mut feeds_on = vec![false; items.len()];
    for l in links.iter().filter(|l| column[l.to] > column[l.from]) {
        feeds_on[l.from] = true;
    }
    // (nodes, the gap after them)
    let mut stacks: Vec<(Vec<usize>, f32)> = Vec::new();
    for col in &best {
        let dealt = deal(col, items, limit, &feeds_on);
        let last = dealt.len() - 1;
        stacks.extend(dealt.into_iter().enumerate().map(|(k, c)| (c, if k == last { COLUMN_GAP } else { DEALT_GAP })));
    }
    let placed: Vec<Vec<usize>> = stacks.iter().map(|s| s.0.clone()).collect();
    stack(&placed, items, &mut y);
    relax(&placed, items, links, &touching, &mut y, PLACE_SWEEPS);

    let up = piece.iter().map(|&n| y[n]).fold(f32::INFINITY, f32::min);
    let down = piece.iter().map(|&n| y[n] + items[n].h).fold(f32::NEG_INFINITY, f32::max);
    let (mut x, mut right) = (0.0f32, 0.0f32);
    for (col, gap) in &stacks {
        for &n in col {
            out[n] = (x, y[n] - up);
        }
        let w = col.iter().map(|&n| items[n].w).fold(0.0, f32::max);
        right = x + w;
        x += w + gap;
    }
    (right, down - up)
}

/// Every column stacked from the top.
fn stack(cols: &[Vec<usize>], items: &[Item], y: &mut [f32]) {
    for col in cols {
        let mut at = 0.0;
        for &n in col {
            y[n] = at;
            at += items[n].h + NODE_GAP;
        }
    }
}

/// Sweeps the columns one way and the other, each node going where its wires want it.
fn relax(cols: &[Vec<usize>], items: &[Item], links: &[Link], touching: &[Vec<usize>], y: &mut [f32], sweeps: usize) {
    for sweep in 0..sweeps {
        let k = cols.len();
        for c in 0..k {
            settle(&cols[if sweep % 2 == 0 { c } else { k - 1 - c }], items, links, touching, y);
        }
    }
}

/// Where node `n`'s top would make its wires level, on average, and how many wires say so. The
/// mean, not the median: a recording and the Time node it feeds would otherwise agree with each
/// other and drift away from the row they both lead to.
fn wanted(n: usize, links: &[Link], touching: &[Vec<usize>], y: &[f32]) -> (f32, f32) {
    let mut sum = 0.0;
    for &k in &touching[n] {
        let l = &links[k];
        sum += if l.from == n { y[l.to] + l.to_y - l.from_y } else { y[l.from] + l.from_y - l.to_y };
    }
    let count = touching[n].len();
    if count == 0 {
        (y[n], 1.0)
    } else {
        (sum / count as f32, count as f32)
    }
}

/// Moves a column's nodes as near as they can get to where their wires want them, keeping the
/// order and the gaps: weighted isotonic regression by pooling adjacent violators.
fn settle(col: &[usize], items: &[Item], links: &[Link], touching: &[Vec<usize>], y: &mut [f32]) {
    // Each pool is (sum of weight × target, sum of weight, nodes in it).
    let mut pools: Vec<(f32, f32, usize)> = Vec::with_capacity(col.len());
    let mut offset = Vec::with_capacity(col.len());
    let mut stacked = 0.0;
    for &n in col {
        let (want, weight) = wanted(n, links, touching, y);
        offset.push(stacked);
        pools.push((weight * (want - stacked), weight, 1));
        stacked += items[n].h + NODE_GAP;
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

/// How many pairs of wires cross, taking each wire as straight from column to column.
fn crossings(links: &[Link], column: &[usize], y: &[f32]) -> usize {
    // (left column, height there, right column, height there)
    let spans: Vec<(f32, f32, f32, f32)> = links
        .iter()
        .filter(|l| column[l.from] != column[l.to])
        .map(|l| {
            let (a, b) = ((column[l.from] as f32, y[l.from] + l.from_y), (column[l.to] as f32, y[l.to] + l.to_y));
            if a.0 < b.0 {
                (a.0, a.1, b.0, b.1)
            } else {
                (b.0, b.1, a.0, a.1)
            }
        })
        .collect();
    let at = |s: &(f32, f32, f32, f32), c: f32| s.1 + (s.3 - s.1) * (c - s.0) / (s.2 - s.0);
    let mut n = 0;
    for (k, a) in spans.iter().enumerate() {
        for b in &spans[k + 1..] {
            let (lo, hi) = (a.0.max(b.0), a.2.min(b.2));
            if lo < hi && (at(a, lo) - at(b, lo)) * (at(a, hi) - at(b, hi)) < 0.0 {
                n += 1;
            }
        }
    }
    n
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
