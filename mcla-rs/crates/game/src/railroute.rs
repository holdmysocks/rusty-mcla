//! The railnet route finder (docs/reference/ai/railnet_route.md): the lane route F-94883 makes for a cruising car's ghost
//! from its rail to its next destination. A best-first search (F-39378) over the navsys records (roads and junctions)
//! entered from a neighbour, g = the summed squared distances of the records' centres, h = the squared distance to the
//! destination record's, an unsorted open array and a closed list of 200; the route keeps per record the lanes that
//! lead on and a "usable" flag per lane. Built on the raw `.r66` (the records, the connection list, the vertex -> lanes
//! table) and the port's lane curves.

use crate::physics::exact::fma;
use crate::vehicles::traffic::{curve_nearest, snap_to_lane};
use glam::Vec3;
use mcla_formats::rage::navsys::NavSys;
use mcla_formats::rsc::Resource;

/// The closed list's size (F-39378: 200 nodes, then no route).
pub const CLOSED_MAX: usize = 200;
/// A node keeps at most 16 lanes (F-47728).
pub const NODE_LANES: usize = 16;
/// The destination is snapped onto the nearest lane within 5 m (F-94883).
pub const SNAP_RADIUS: f32 = 5.0;
/// F-47728: a road this fast (m/s, 29.0129: the 65 mph roads) lets a lane whose start runs on from the previous record's first lane
/// go without the reachability test.
const FAST_ROAD: f32 = f32::from_bits(0x41E8_1A69);

/// A record of the navsys (a road, flags +4 & 2, or a junction, & 4), by its id +32.
#[derive(Clone, Debug, Default)]
pub struct Rec {
    pub flags: u32,
    /// +0x50: the centre of its rectangle (x, 0, z)
    pub c: Vec3,
    /// +148: the speed limit in m/s (0 = the default)
    pub speed: f32,
    /// the connection list (root +0x3c) in file order, as record ids
    pub next: Vec<u32>,
    /// its lanes as F-12622 indexes them (F-95725 counts them): its own (+20 / +24), then its first child's (+12)
    /// subtree, each level its own lanes, its next sibling's (+16) subtree, then its child's
    pub lanes: Vec<u32>,
}

/// A lane's part of the graph.
#[derive(Clone, Debug, Default)]
pub struct LaneNode {
    /// F-17030: the record of the lane (the owner or its parent, a road first, then a junction)
    pub rec: Option<u32>,
    /// the owner's flags +4 and whether the owner has no next sibling (+16 == 0)
    pub owner_flags: u32,
    pub owner_last: bool,
    /// the vertex indices of its first and last control point (curve +32)
    pub v0: u32,
    pub v1: u32,
}

/// The railnet as the route search reads it.
#[derive(Clone, Debug, Default)]
pub struct RailNet {
    pub recs: Vec<Rec>,
    pub lanes: Vec<LaneNode>,
    /// root +0x38: per vertex the lanes touching it, in table order
    pub vert: Vec<Vec<u32>>,
    /// railnet +69 (F-45962 sets 1): the lane walks stay in the records they are asked about
    pub stay: bool,
}

impl RailNet {
    /// The tables from the navsys resource (lanes in file order, the order of `NavSys::curves`).
    pub fn parse(res: &Resource) -> RailNet {
        let d = &res.data;
        let vs = res.virtual_size;
        let be = |o: usize| u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]);
        let h16 = |o: usize| u16::from_be_bytes([d[o], d[o + 1]]) as usize;
        let fl = |o: usize| f32::from_bits(be(o));
        let ptr = |o: usize| res.ptr(be(o));
        let mut lane_offs = Vec::new();
        let mut o = 0usize;
        while o + 0x30 <= vs {
            match u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) {
                0x0047_9f0c => {
                    lane_offs.push(o);
                    o += 0x70;
                }
                0x0047_a13c => o += 0xb0,
                0x0047_a0c4 => o += 0xc0,
                _ => o += 4,
            }
        }
        let lane_of = |off: usize| lane_offs.binary_search(&off).ok().map(|i| i as u32);
        let id = |r: usize| be(r + 32);
        // F-17030
        let rec_of = |lane: usize| -> Option<usize> {
            let ow = ptr(lane + 8)?;
            let par = ptr(ow + 8);
            if be(ow + 4) & 2 != 0 {
                return Some(ow);
            }
            if let Some(p) = par.filter(|&p| be(p + 4) & 2 != 0) {
                return Some(p);
            }
            if be(ow + 4) & 4 != 0 {
                return Some(ow);
            }
            par.filter(|&p| be(p + 4) & 4 != 0)
        };
        fn walk(o: usize, sib: bool, out: &mut Vec<u32>, own: &dyn Fn(usize, &mut Vec<u32>), child: &dyn Fn(usize) -> (Option<usize>, Option<usize>)) {
            own(o, out);
            let (c, s) = child(o);
            if sib {
                if let Some(s) = s {
                    walk(s, true, out, own, child);
                }
            }
            if let Some(c) = c {
                walk(c, true, out, own, child);
            }
        }
        let own = |o: usize, out: &mut Vec<u32>| {
            if let Some(a) = ptr(o + 20) {
                for k in 0..h16(o + 24) {
                    if let Some(l) = ptr(a + 4 * k).and_then(lane_of) {
                        out.push(l);
                    }
                }
            }
        };
        let child = |o: usize| (ptr(o + 12), ptr(o + 16));
        let conn = ptr(0x3c);
        let conn_n = be(0x4c) as usize;
        let mut recs: Vec<Rec> = vec![Rec::default(); conn_n];
        let mut seen = vec![false; conn_n];
        let mut add = |r: usize, recs: &mut Vec<Rec>| {
            let i = id(r) as usize;
            if i >= recs.len() {
                recs.resize(i + 1, Rec::default());
                seen.resize(i + 1, false);
            }
            if seen[i] {
                return;
            }
            seen[i] = true;
            let mut lanes = Vec::new();
            walk(r, false, &mut lanes, &own, &child);
            let mut next = Vec::new();
            if let Some(e) = conn.filter(|_| i < conn_n).map(|c| c + 16 * i) {
                if let Some(a) = ptr(e) {
                    for k in 0..h16(e + 4) {
                        if let Some(n) = ptr(a + 4 * k) {
                            next.push(id(n));
                        }
                    }
                }
            }
            recs[i] = Rec { flags: be(r + 4), c: Vec3::new(fl(r + 0x50), fl(r + 0x54), fl(r + 0x58)), speed: fl(r + 148), next, lanes };
        };
        let mut lanes = Vec::with_capacity(lane_offs.len());
        let mut pending = Vec::new();
        for &lo in &lane_offs {
            let r = rec_of(lo);
            if let Some(r) = r {
                pending.push(r);
            }
            let ow = ptr(lo + 8);
            let n = h16(lo + 0x4c);
            let vi = ptr(lo + 0x50);
            let v = |k: usize| vi.map_or(0, |a| h16(a + 2 * k) as u32);
            lanes.push(LaneNode { rec: r.map(id), owner_flags: ow.map_or(0, |w| be(w + 4)), owner_last: ow.is_some_and(|w| be(w + 16) == 0), v0: v(0), v1: v(n) });
        }
        if let Some(c) = conn {
            for i in 0..conn_n {
                if let Some(a) = ptr(c + 16 * i) {
                    for k in 0..h16(c + 16 * i + 4) {
                        if let Some(n) = ptr(a + 4 * k) {
                            pending.push(n);
                        }
                    }
                }
            }
        }
        for r in pending {
            add(r, &mut recs);
        }
        let mut vert = Vec::new();
        if let Some(t) = ptr(0x38) {
            let nv = lanes.iter().map(|l: &LaneNode| l.v0.max(l.v1) as usize + 1).max().unwrap_or(0);
            for v in 0..nv {
                let e = t + 8 * v;
                let mut ls = Vec::new();
                if let Some(a) = ptr(e) {
                    for k in 0..h16(e + 4) {
                        if let Some(l) = ptr(a + 4 * k).and_then(lane_of) {
                            ls.push(l);
                        }
                    }
                }
                vert.push(ls);
            }
        }
        RailNet { recs, lanes, vert, stay: true }
    }

    fn rec(&self, id: u32) -> &Rec {
        &self.recs[id as usize]
    }
}

/// A node of the search and of the route (F-26157's 120-byte node): the record, the record it was entered from, g, h,
/// the lanes and, in a route, their usable flags (+100).
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub rec: u32,
    pub from: Option<u32>,
    pub g: f32,
    pub h: f32,
    pub lanes: Vec<u32>,
    pub usable: Vec<bool>,
    parent: Option<usize>,
}

/// What F-94883 hands back: the snapped destination and the route (empty: none).
#[derive(Clone, Debug, Default)]
pub struct Route {
    pub dest_lane: Option<u32>,
    pub dest_t: f32,
    pub dest_point: Vec3,
    /// the destination lane's direction there, normalised (S+64)
    pub dest_dir: Vec3,
    pub nodes: Vec<Node>,
    /// the closed list's size when the search stopped (200: the limit)
    pub closed: usize,
}

/// The search's inputs (F-94883's record S, the parts the cruising params leave in play).
struct Search<'a> {
    net: &'a RailNet,
    nav: &'a NavSys,
    start: u32,
    start_t: f32,
    dest: u32,
    dest_t: f32,
}

/// The squared distance of two records' centres as the cost writes it (F-43730): dy^2, then dz fused, then dx fused.
fn d2(a: Vec3, b: Vec3) -> f32 {
    let (dy, dz, dx) = (a.y - b.y, a.z - b.z, a.x - b.x);
    fma(dx, dx, fma(dz, dz, dy * dy))
}

/// F-71868 by way of lane slot 8 (F-32383): the point and the derivative at t.
fn pt_dir(nav: &NavSys, lane: u32, t: f32) -> (Vec3, Vec3) {
    nav.curves[lane as usize].eval_pair(t)
}

/// The vector unit's normalisation (as F-94883 / F-16467 do it): x = (0 + z^2) + (y^2 + x^2), the reciprocal square
/// root (1 / sqrt; +inf -> 0), one Newton step y + (1 - x y^2)(0.5 y), not fused.
pub fn vnorm(v: Vec3) -> Vec3 {
    let x = v.z * v.z + (v.y * v.y + v.x * v.x);
    let mut y = 1.0 / x.sqrt();
    if y == f32::INFINITY {
        y = 0.0;
    }
    let e = -(x * (y * y) - 1.0);
    let s = e * (0.5 * y) + y;
    v * s
}

impl<'a> Search<'a> {
    fn cv(&self, lane: u32, i: usize) -> Vec3 {
        self.nav.curves[lane as usize].pts[i]
    }
    fn first(&self, lane: u32) -> Vec3 {
        self.cv(lane, 0)
    }
    fn last(&self, lane: u32) -> Vec3 {
        let p = &self.nav.curves[lane as usize].pts;
        p[p.len() - 1]
    }

    /// F-91078: the lane leads (fwd) to / comes (back) from record `target` through at most 5 lanes joined end to start
    /// (exact xyz), the lanes of its end / start vertex in table order; with railnet +69 the walk stays in `rec`.
    fn reaches_rec(&self, lane: u32, rec: u32, target: u32, fwd: bool, depth: u32) -> bool {
        let r = self.net.lanes[lane as usize].rec;
        if r == Some(target) {
            return true;
        }
        if self.net.stay && r != Some(rec) {
            return false;
        }
        if depth > 5 {
            return false;
        }
        let ln = &self.net.lanes[lane as usize];
        let v = if fwd { ln.v1 } else { ln.v0 };
        let Some(list) = self.net.vert.get(v as usize) else { return false };
        for &o in list {
            if o == lane {
                continue;
            }
            let joined = if fwd { self.first(o) == self.last(lane) } else { self.last(o) == self.first(lane) };
            if joined && self.reaches_rec(o, rec, target, fwd, depth + 1) {
                return true;
            }
        }
        false
    }

    /// F-28632: the lane leads to / comes from lane `target` the same way, staying in `ra` or `rb` with railnet +69.
    fn reaches_lane(&self, lane: u32, ra: u32, target: u32, rb: u32, fwd: bool, depth: u32) -> bool {
        if lane == target {
            return true;
        }
        let r = self.net.lanes[lane as usize].rec;
        if self.net.stay && r != Some(ra) && r != Some(rb) {
            return false;
        }
        if depth > 5 {
            return false;
        }
        let ln = &self.net.lanes[lane as usize];
        let v = if fwd { ln.v1 } else { ln.v0 };
        let Some(list) = self.net.vert.get(v as usize) else { return false };
        for &o in list {
            if o == lane {
                continue;
            }
            let joined = if fwd { self.first(o) == self.last(lane) } else { self.last(o) == self.first(lane) };
            if joined && self.reaches_lane(o, ra, target, rb, fwd, depth + 1) {
                return true;
            }
        }
        false
    }

    /// The lanes of the lane's record other than itself onto which `p` falls inside them (F-24415, 4 chords: 0 < phase
    /// < 1) with their direction there (lane slot 10 F-26650) not against `dir` in x / z; (lane, phase, point).
    fn beside(&self, lane: u32, p: Vec3) -> Vec<(u32, f32, Vec3, Vec3)> {
        let Some(r) = self.net.lanes[lane as usize].rec else { return Vec::new() };
        let mut out = Vec::new();
        for &o in &self.net.rec(r).lanes {
            if o == lane {
                continue;
            }
            let (ph, _, q) = curve_nearest(&self.nav.curves[o as usize], p, 4, 10);
            if !(ph > 0.0) || !(ph < 1.0) {
                continue;
            }
            out.push((o, ph, q, pt_dir(self.nav, o, ph).1));
        }
        out
    }

    /// F-14354: the lane at t, then the record's other lanes the point falls on whose direction is not against the
    /// lane's (xz: dir.z x d.z, then d.x x dir.x fused, >= 0), each with its phase.
    fn same_way(&self, lane: u32, t: f32) -> Vec<(u32, f32)> {
        let (p, dir) = pt_dir(self.nav, lane, t);
        let mut out = vec![(lane, t)];
        for (o, ph, _, d) in self.beside(lane, p) {
            let dot = fma(d.x, dir.x, dir.z * d.z);
            if dot < 0.0 {
                continue;
            }
            out.push((o, ph));
        }
        out
    }

    /// F-16467: the lane at t and its nearest same-way neighbour on each side - side = (q - p) . (dir.z, 0, -dir.x) with
    /// dir the xz direction normalised; s >= 0 the least (right), s < 0 the greatest (left); the right one first.
    fn sides(&self, lane: u32, t: f32) -> Vec<(u32, f32)> {
        let (p, dir) = pt_dir(self.nav, lane, t);
        let n = vnorm(Vec3::new(dir.x, 0.0, dir.z));
        let (mut right, mut left) = (None, None);
        let (mut br, mut bl) = (f32::MAX, -f32::MAX);
        for (o, ph, q, d) in self.beside(lane, p) {
            let dot = fma(d.z, n.z, d.x * n.x);
            if dot < 0.0 {
                continue;
            }
            let v = q - p;
            let s = fma(v.z, -n.x, v.x * n.z);
            if !(s < 0.0) {
                if !(s >= br) {
                    right = Some((o, ph));
                    br = s;
                }
            } else if s > bl {
                left = Some((o, ph));
                bl = s;
            }
        }
        let mut out = vec![(lane, t)];
        out.extend(right);
        out.extend(left);
        out
    }

    /// F-88732: the goal pairs (record, entered from). The same record at both ends: when a start lane meets a
    /// destination lane ahead (same lane, s.t <= d.t) or reaches it (F-28632), the start node itself. Otherwise per
    /// neighbour N of the destination record (in order) the pair (dest, N) when a destination lane comes from N.
    fn goals(&self) -> Vec<(u32, Option<u32>)> {
        let d = self.same_way(self.dest, self.dest_t);
        let rs = self.net.lanes[self.start as usize].rec;
        let rd = self.net.lanes[self.dest as usize].rec;
        let Some(rdi) = rd else { return Vec::new() };
        if rs == rd {
            let s2 = self.same_way(self.start, self.start_t);
            let mut found = false;
            'o: for s in &s2 {
                for e in &d {
                    if found {
                        break 'o;
                    }
                    found = if s.0 == e.0 { s.1 <= e.1 } else { self.reaches_lane(s.0, rdi, e.0, rdi, true, 0) };
                }
                if found {
                    break;
                }
            }
            if found {
                return vec![(rdi, None)];
            }
        }
        let mut out = Vec::new();
        for &n in &self.net.rec(rdi).next {
            for e in &d {
                let er = self.net.lanes[e.0 as usize].rec.unwrap_or(u32::MAX);
                if self.reaches_rec(e.0, er, n, false, 0) {
                    out.push((rdi, Some(n)));
                    break;
                }
            }
        }
        out
    }

    /// F-87157: a record's speed: +148 (0: 40 mph) plus a global mph offset (0 in the image) x 0.44704, x 1.0.
    fn speed(&self, r: u32) -> f32 {
        let s = self.net.rec(r).speed;
        let k = f32::from_bits(0x3EE4_E26D);
        let base = if s == 0.0 { 40.0 * k } else { s };
        fma(0.0, k, base) * 1.0
    }

    /// F-47728: the successors of (R, from F): per neighbour N of R in order (the start node: only those F-48700 lets
    /// through) a node (N, from R) with the lanes of R that lead to N (F-91078) and, after a record F, come from F -
    /// unless R is a fast road or F's first lane's owner is a last 0x20 owner, and the lane starts the way F's first
    /// lane ends (dot > 0). Nodes without a lane are dropped; at most 16 lanes.
    fn expand(&self, rec: u32, from: Option<u32>) -> Vec<Node> {
        let r = self.net.rec(rec);
        let entries = if from.is_none() { self.sides(self.start, self.start_t) } else { Vec::new() };
        let mut out = Vec::new();
        for &n in &r.next {
            if from.is_none() && !self.start_ok(n, &entries) {
                continue;
            }
            let mut node = Node { rec: n, from: Some(rec), g: 0.0, h: 0.0, lanes: Vec::new(), usable: Vec::new(), parent: None };
            for &l in &r.lanes {
                if !self.reaches_rec(l, rec, n, true, 0) {
                    continue;
                }
                if let Some(f) = from {
                    let fl = self.net.rec(f).lanes.first().copied();
                    let owner = fl.map(|x| &self.net.lanes[x as usize]);
                    let cond_a = owner.is_some_and(|o| o.owner_flags & 0x20 != 0 && o.owner_last);
                    let cond_b = r.flags & 2 != 0 && self.speed(rec) >= FAST_ROAD;
                    let mut check = true;
                    if let Some(f0) = fl.filter(|_| cond_b || cond_a) {
                        let np = self.nav.curves[f0 as usize].pts.len();
                        let d1 = self.last(f0) - self.cv(f0, np.saturating_sub(2));
                        let d2v = self.cv(l, 1) - self.first(l);
                        let dot = fma(d1.x, d2v.x, fma(d1.y, d2v.y, d1.z * d2v.z));
                        check = !(dot > 0.0);
                    }
                    if check && !self.reaches_rec(l, rec, f, false, 0) {
                        continue;
                    }
                }
                if node.lanes.len() < NODE_LANES {
                    node.lanes.push(l);
                }
            }
            if !node.lanes.is_empty() {
                out.push(node);
            }
        }
        out
    }

    /// F-48700: from the start: a junction connector start lane (owner flags & 0x30) must lead to N itself, else one of
    /// the entry lanes (F-16467).
    fn start_ok(&self, n: u32, entries: &[(u32, f32)]) -> bool {
        let s = &self.net.lanes[self.start as usize];
        if s.owner_flags & 0x30 != 0 {
            return self.reaches_rec(self.start, s.rec.unwrap_or(u32::MAX), n, true, 0);
        }
        entries.iter().any(|e| self.reaches_rec(e.0, self.net.lanes[e.0 as usize].rec.unwrap_or(u32::MAX), n, true, 0))
    }

    /// F-39378 and the route build F-75309; None: no goal pair, the closed list full, the open list empty.
    fn run(&self) -> (Option<Vec<Node>>, usize) {
        let pairs = self.goals();
        if pairs.is_empty() {
            return (None, 0);
        }
        let rs = self.net.lanes[self.start as usize].rec.unwrap();
        let target = self.net.rec(pairs[0].0).c;
        let rd = self.net.lanes[self.dest as usize].rec.unwrap();
        let mut arena = vec![Node { rec: rs, from: None, g: 0.0, h: d2(self.net.rec(rs).c, self.net.rec(rd).c), lanes: Vec::new(), usable: Vec::new(), parent: None }];
        let mut open: Vec<usize> = vec![0];
        let mut closed: Vec<usize> = Vec::new();
        let same = |a: &Node, b: &Node| a.rec == b.rec && a.from == b.from;
        let found = loop {
            // F-69875: the least h + g below FLT_MAX, the first of equals; the last one moved into its slot
            let mut best = f32::MAX;
            let mut bi = None;
            for (i, &k) in open.iter().enumerate() {
                let f = arena[k].h + arena[k].g;
                if f < best {
                    best = f;
                    bi = Some(i);
                }
            }
            let Some(bi) = bi else { break None };
            let cur = open.swap_remove(bi);
            // F-35460
            if pairs.iter().any(|p| p.0 == arena[cur].rec && p.1 == arena[cur].from) {
                break Some(cur);
            }
            let (crec, cfrom, cg) = (arena[cur].rec, arena[cur].from, arena[cur].g);
            for mut s in self.expand(crec, cfrom) {
                // F-43730 (the cruising params: no penalty, no record cost)
                let c = self.net.rec(s.rec).c;
                s.g = cg + d2(self.net.rec(crec).c, c);
                s.h = d2(c, target);
                // F-64549 in both lists: an equal node at least as cheap keeps its place
                let io = open.iter().position(|&k| same(&arena[k], &s));
                if io.is_some_and(|i| !(s.g < arena[open[i]].g)) {
                    continue;
                }
                let ic = closed.iter().position(|&k| same(&arena[k], &s));
                if ic.is_some_and(|i| !(s.g < arena[closed[i]].g)) {
                    continue;
                }
                for (idx, list_is_open) in [(io, true), (ic, false)] {
                    let Some(i) = idx else { continue };
                    let old = if list_is_open { open[i] } else { closed[i] };
                    for &k in open.iter().chain(closed.iter()) {
                        if arena[k].parent == Some(old) {
                            arena[k].parent = None;
                        }
                    }
                    if list_is_open {
                        open.swap_remove(i);
                    } else {
                        closed.swap_remove(i);
                    }
                }
                s.parent = Some(cur);
                arena.push(s);
                open.push(arena.len() - 1);
            }
            closed.push(cur);
            if closed.len() == CLOSED_MAX || open.is_empty() {
                break None;
            }
        };
        let Some(end) = found else { return (None, closed.len()) };
        let mut chain = vec![end];
        while let Some(p) = arena[*chain.last().unwrap()].parent {
            chain.push(p);
        }
        chain.reverse();
        let mut nodes: Vec<Node> = chain.into_iter().map(|k| arena[k].clone()).collect();
        self.build(&mut nodes);
        (Some(nodes), closed.len())
    }

    /// F-75309: F-99266 (each node takes the next one's lanes - its own record's lanes into the next record; the last
    /// none), F-85997 (the last node's lanes), F-16433 (the usable flags).
    fn build(&self, nodes: &mut [Node]) {
        let n = nodes.len();
        for i in 0..n {
            nodes[i].lanes = if i + 1 < n { nodes[i + 1].lanes.clone() } else { Vec::new() };
        }
        // F-85997: the last record's lanes that are the destination lane or reach it in the record, and come from a lane
        // of the node before (when it has any)
        let e = nodes[n - 1].rec;
        let (plan, q) = if n > 1 { (nodes[n - 2].lanes.clone(), nodes[n - 2].rec) } else { (Vec::new(), 0) };
        let mut last = Vec::new();
        for &l in &self.net.rec(e).lanes {
            let keep = l == self.dest || (self.reaches_lane(l, e, self.dest, e, true, 0) && (plan.is_empty() || plan.iter().any(|&p| self.reaches_lane(l, e, p, q, false, 0))));
            if keep {
                last.push(l);
            }
        }
        nodes[n - 1].lanes = last;
        // F-16433
        for (i, nd) in nodes.iter_mut().enumerate() {
            nd.usable = vec![false; nd.lanes.len()];
            if nd.lanes.len() == 1 {
                nd.usable[0] = true;
            }
            if i == n - 1 {
                if let Some(k) = nd.lanes.iter().position(|&l| l == self.dest) {
                    nd.usable[k] = true;
                }
            }
        }
        for i in 0..n.saturating_sub(1) {
            for a in 0..nodes[i].lanes.len() {
                if nodes[i].usable[a] {
                    continue;
                }
                let end = self.last(nodes[i].lanes[a]);
                if nodes[i + 1].lanes.iter().any(|&b| self.first(b) == end) {
                    nodes[i].usable[a] = true;
                }
            }
        }
        for nd in nodes.iter_mut() {
            for a in 0..nd.lanes.len() {
                if nd.usable[a] {
                    continue;
                }
                let end = self.last(nd.lanes[a]);
                for b in 0..nd.lanes.len() {
                    if nd.usable[b] && self.first(nd.lanes[b]) == end {
                        nd.usable[a] = true;
                        break;
                    }
                }
            }
        }
    }
}

/// The search alone (F-39378 with the route build) from a start lane at t to a destination lane at t, with the
/// cruising brain's static route params (no option on). Returns the route's nodes (None: no route) and the closed list's size.
pub fn search(net: &RailNet, nav: &NavSys, start: u32, start_t: f32, dest: u32, dest_t: f32) -> (Option<Vec<Node>>, usize) {
    if net.lanes[start as usize].rec.is_none() || net.lanes[dest as usize].rec.is_none() {
        return (None, 0);
    }
    Search { net, nav, start, start_t, dest, dest_t }.run()
}

/// F-94883: the route from a start lane at t to a point - the point snapped onto the nearest lane within 5 m (F-59579
/// without a heading), its direction normalised, then the search.
pub fn route(net: &RailNet, nav: &NavSys, start: u32, start_t: f32, dest: Vec3) -> Route {
    let (hit, _) = snap_to_lane(nav, dest, SNAP_RADIUS);
    let Some(h) = hit else { return Route::default() };
    let lane = h.lane as u32;
    let (nodes, closed) = search(net, nav, start, start_t, lane, h.t);
    Route { dest_lane: Some(lane), dest_t: h.t, dest_point: h.point, dest_dir: vnorm(pt_dir(nav, lane, h.t).1), nodes: nodes.unwrap_or_default(), closed }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Case {
        sl: u32,
        t: f32,
        d: Vec3,
        s: i64,
        e: i64,
        dl: i64,
        dt: f32,
        p: Vec3,
        tg: Vec3,
        nodes: Vec<(i64, i64, u32, u32, Vec<(i64, bool)>)>,
        closed: i64,
    }

    fn parse(text: &str) -> Vec<Case> {
        let f = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let mut out = Vec::new();
        for line in text.lines().filter(|l| l.starts_with("C ")) {
            let (head, tail) = line.split_once(" | vc ").unwrap();
            let parts: Vec<&str> = head.split(" | ").collect();
            let a: Vec<&str> = parts[0].split(' ').collect();
            let b: Vec<&str> = parts[2].split(' ').collect();
            let mut nodes = Vec::new();
            for nd in parts.get(4).copied().unwrap_or("").split('[').skip(1) {
                let w: Vec<&str> = nd.trim_end_matches(|c| c == ']' || c == ' ').split(' ').collect();
                let (rec, from) = w[0].split_once('<').unwrap();
                let lanes = w[5..].iter().map(|x| (x.trim_end_matches('*').parse::<i64>().unwrap(), x.ends_with('*'))).collect();
                nodes.push((rec.parse().unwrap(), from.parse().unwrap(), u32::from_str_radix(w[2], 16).unwrap(), u32::from_str_radix(w[4], 16).unwrap(), lanes));
            }
            let tail: Vec<&str> = tail.split(' ').collect();
            out.push(Case {
                sl: a[3].parse().unwrap(),
                t: f(a[5]),
                d: Vec3::new(f(a[7]), f(a[8]), f(a[9])),
                s: b[1].parse().unwrap(),
                e: b[3].parse().unwrap(),
                dl: b[5].parse().unwrap(),
                dt: f(b[7]),
                p: Vec3::new(f(b[9]), f(b[10]), f(b[11])),
                tg: Vec3::new(f(b[13]), f(b[14]), f(b[15])),
                nodes,
                closed: tail[2].parse().unwrap(),
            });
        }
        out
    }

    fn load() -> Option<(NavSys, RailNet)> {
        let root = std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into()));
        let res = crate::assets::load_resource_file(&root.join("resources/city/sc/0x9960fcd8.r66")).ok()?;
        let nav = NavSys::parse(&res).ok()?;
        let net = RailNet::parse(&res);
        Some((nav, net))
    }

    /// Per case: [search equal, snap equal (lane, t, point, direction)] counts and the first differing search cases.
    fn check(text: &str) -> (usize, usize, usize, usize, Vec<String>) {
        let Some((nav, net)) = load() else { return (0, 0, 0, 0, Vec::new()) };
        let cases = parse(text);
        let (mut searched, mut equal, mut snaps, mut snap_eq) = (0, 0, 0, 0);
        let mut bad = Vec::new();
        let mut parts = [0usize; 4];
        for (k, c) in cases.iter().enumerate() {
            let r = route(&net, &nav, c.sl, c.t, c.d);
            snaps += 1;
            let want_lane = (c.dl >= 0).then_some(c.dl as u32);
            if r.dest_lane == want_lane && want_lane.is_some() {
                parts[0] += 1;
                parts[1] += (r.dest_t.to_bits() == c.dt.to_bits()) as usize;
                parts[2] += (r.dest_point.to_array().map(f32::to_bits) == c.p.to_array().map(f32::to_bits)) as usize;
                parts[3] += (r.dest_dir.to_array().map(f32::to_bits) == c.tg.to_array().map(f32::to_bits)) as usize;
            }
            if r.dest_lane == want_lane && (want_lane.is_none() || (r.dest_t.to_bits() == c.dt.to_bits() && r.dest_point.to_array().map(f32::to_bits) == c.p.to_array().map(f32::to_bits) && r.dest_dir.to_array().map(f32::to_bits) == c.tg.to_array().map(f32::to_bits))) {
                snap_eq += 1;
            }
            let Some(dl) = want_lane else { continue };
            searched += 1;
            let rid = |l: u32| net.lanes[l as usize].rec.map_or(-1, |x| x as i64);
            let (nodes, closed) = search(&net, &nav, c.sl, c.t, dl, c.dt);
            let got: Vec<(i64, i64, u32, u32, Vec<(i64, bool)>)> = nodes
                .unwrap_or_default()
                .iter()
                .map(|n| (n.rec as i64, n.from.map_or(-1, |x| x as i64), n.g.to_bits(), n.h.to_bits(), n.lanes.iter().zip(&n.usable).map(|(&l, &u)| (l as i64, u)).collect()))
                .collect();
            let ok = got == c.nodes && rid(c.sl) == c.s && rid(dl) == c.e && (!got.is_empty() || c.closed < 0 || closed as i64 == c.closed);
            if ok {
                equal += 1;
            } else if bad.len() < 8 {
                let first = got.iter().zip(&c.nodes).position(|(a, b)| a != b);
                bad.push(format!("case {k}: port {} nodes (closed {closed}), original {} (closed {}); first differing node {:?}: port {:?} original {:?}", got.len(), c.nodes.len(), c.closed, first, first.and_then(|i| got.get(i)), first.and_then(|i| c.nodes.get(i))));
            }
        }
        bad.push(format!("snap: same lane {} (t equal {}, point equal {}, direction equal {})", parts[0], parts[1], parts[2], parts[3]));
        (searched, equal, snaps, snap_eq, bad)
    }

    /// F-94883 / F-39378 against the original's code run offline on the real railnet (work/racerai/route, 120 cases of
    /// out_small.txt): the search from the original's snapped destination - every node's record, entered-from record,
    /// g and h bits, lanes and usable flags, the fail cases' closed count; and the port's own 5 m snap.
    #[test]
    fn route_matches_the_original() {
        let (searched, equal, snaps, snap_eq, bad) = check(include_str!("testdata/railroute_small.txt"));
        if searched == 0 {
            return;
        }
        println!("railroute small: search {equal} / {searched} equal, snap {snap_eq} / {snaps} equal");
        for b in &bad {
            println!("{b}");
        }
        assert_eq!(equal, searched, "search");
    }

    /// The record lane order F-12622 walks (the route test passes with it) against the port's `junction_lanes` and
    /// `road_lanes`: prints how many records list the same lanes in the same order.
    #[test]
    fn record_lane_order_against_navsys() {
        let Some((nav, net)) = load() else { return };
        let (mut j_same, mut j_set, mut j_n) = (0, 0, 0);
        for jl in &nav.junction_lanes {
            let Some(&l0) = jl.first() else { continue };
            let Some(r) = net.lanes[l0].rec else { continue };
            j_n += 1;
            let mine: Vec<usize> = net.recs[r as usize].lanes.iter().map(|&x| x as usize).collect();
            j_same += (mine == *jl) as usize;
            let (mut a, mut b) = (mine.clone(), jl.clone());
            a.sort();
            b.sort();
            j_set += (a == b) as usize;
        }
        let (mut r_same, mut r_n) = (0, 0);
        for road in 0..nav.zones.len() {
            let rl = nav.road_lanes(road);
            let Some(&l0) = rl.first() else { continue };
            let Some(r) = net.lanes[l0].rec else { continue };
            r_n += 1;
            r_same += net.recs[r as usize].lanes.iter().map(|&x| x as usize).eq(rl.iter().copied()) as usize;
        }
        println!("record lanes: junctions same order {j_same} / {j_n} (same set {j_set}), roads same order {r_same} / {r_n}");
    }

    /// The same on the 20,000 cases of work/racerai/route/out_route.txt when it is there.
    #[test]
    fn route_matches_the_original_20000() {
        let p = std::env::var("MCLA_RAILROUTE_REF").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../work/racerai/route/out_route.txt"));
        let Ok(text) = std::fs::read_to_string(p) else { return };
        let (searched, equal, snaps, snap_eq, bad) = check(&text);
        if searched == 0 {
            return;
        }
        println!("railroute 20000: search {equal} / {searched} equal, snap {snap_eq} / {snaps} equal");
        for b in &bad {
            println!("{b}");
        }
        assert_eq!(equal, searched, "search");
    }
}
