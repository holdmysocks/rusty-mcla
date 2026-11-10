//! The cruising ghost's route follower (docs/reference/ai/cruise_ghost.md section 4): what mcAmbientBehaviorDrive2 does
//! with the route the cruising brain hands it (railroute::Route, the behaviour's route object) - the node of the current
//! lane (F-18846 / F-74061), the next lane at a change of rail (F-74277: the route's lanes first, F-52442 strict and
//! loose, else the ordinary pick F-20639), the route's end and the off-route test (F-44936, F-20872), and the route lane
//! change (F-24481: the gate F-96498, the targets F-81401 / F-49677 / F-97539, the rail's length F-36745 and width
//! F-67514). Pure functions over the route, the railnet tables and the port's lane curves; the lane-change rail itself
//! (F-32118) is not here.

use crate::physics::exact::{fma, fms, fnms};
use crate::railroute::{Node, RailNet, Route};
use crate::vehicles::parked::Mwc;

use glam::Vec3;
use mcla_formats::rage::navsys::{LaneCurve, NavSys};

/// The route object's two indices (+128, +132): the node of the current lane (-1: none) and the last one found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Track {
    pub cur: i32,
    pub last: i32,
}

impl Default for Track {
    /// As behaviour slot 10 (F-28180) copies a route in (F-47634): both -1.
    fn default() -> Self {
        Track { cur: -1, last: -1 }
    }
}

/// A lane with a phase and a node index (the 12-byte records F-24481 passes about: lane, phase, index).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pin {
    pub lane: Option<u32>,
    pub phase: f32,
    pub idx: i32,
}

impl Pin {
    /// F-24481's initial record: no lane, phase 0, index -1.
    const NONE: Pin = Pin { lane: None, phase: 0.0, idx: -1 };
}

/// A route lane change F-24481 makes: the arguments of the lane-change rail F-32118 (from the current lane at t onto
/// `to` at `phase`, `len` metres long, `w` its second length).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Change {
    pub from: u32,
    pub to: u32,
    pub t: f32,
    pub phase: f32,
    pub len: f32,
    pub w: f32,
}

/// The behaviour's flag byte (B+256): 0x20 the destination speed is set.
pub const FLAG_DEST_SPEED: u8 = 0x20;

/// Lane slot 12 (F-88700): the first control point.
fn start(nav: &NavSys, l: u32) -> Vec3 {
    nav.curves[l as usize].pts[0]
}

/// Lane slot 13 (F-93886): the last control point.
fn end(nav: &NavSys, l: u32) -> Vec3 {
    let p = &nav.curves[l as usize].pts;
    p[p.len() - 1]
}

/// F-49670 with the record test (r6 = 0): the node's record lists the lane (F-95725 / F-61293).
fn holds(net: &RailNet, node: &Node, lane: u32) -> bool {
    net.recs[node.rec as usize].lanes.contains(&lane)
}

/// F-74061 (r6 = r7 = 0): the index of the node whose record holds `lane`, searched from `last`, then the nodes after
/// it, then from the first up to it (`last` < 0: all from the first); -1 for none, no lane or an empty route.
pub fn route_index(route: &Route, net: &RailNet, last: i32, lane: Option<u32>) -> i32 {
    let Some(lane) = lane else { return -1 };
    let n = route.nodes.len() as i32;
    if n == 0 {
        return -1;
    }
    let at = |i: i32| holds(net, &route.nodes[i as usize], lane);
    if last < 0 {
        return (0..n).find(|&i| at(i)).unwrap_or(-1);
    }
    if at(last) {
        return last;
    }
    (last + 1..n).chain(0..last).find(|&i| at(i)).unwrap_or(-1)
}

/// F-18846 (behaviour flag 0x10, the replan, off): the node of the current lane; the last one kept when found.
pub fn track(route: &Route, net: &RailNet, t: &mut Track, lane: Option<u32>) {
    t.cur = route_index(route, net, t.last, lane);
    if t.cur >= 0 {
        t.last = t.cur;
    }
}

/// F-52442: a lane of node `i` that starts where `l` ends. Strict: the node's usable lanes (one by the car's own
/// numbers when several), else its first unusable one; loose: the node's record's lanes, first one whose owner has
/// neither flags 0x30 nor 0x40, else the first.
fn node_next(route: &Route, net: &RailNet, nav: &NavSys, l: u32, car: &mut Mwc, i: i32, strict: bool) -> Option<u32> {
    if i < 0 || i >= route.nodes.len() as i32 {
        return None;
    }
    let node = &route.nodes[i as usize];
    let e = end(nav, l);
    if strict {
        let hits: Vec<u32> = node.lanes.iter().zip(&node.usable).filter(|&(&o, &u)| u && start(nav, o) == e).map(|(&o, _)| o).collect();
        if !hits.is_empty() {
            return Some(hits[car.index(hits.len() as u32) as usize]);
        }
        return node.lanes.iter().zip(&node.usable).find(|&(&o, &u)| !u && start(nav, o) == e).map(|(&o, _)| o);
    }
    let lanes = &net.recs[node.rec as usize].lanes;
    lanes
        .iter()
        .copied()
        .find(|&o| start(nav, o) == e && net.lanes[o as usize].owner_flags & 0x70 == 0)
        .or_else(|| lanes.iter().copied().find(|&o| start(nav, o) == e))
}

/// F-20639 with the mask byte 0xE0 (no owner filter): a lane other than `l` at its end vertex (root +0x38, in table
/// order) that starts where `l` ends, by the shared numbers; none: no draw.
pub fn ordinary_next(net: &RailNet, nav: &NavSys, l: u32, shared: &mut Mwc) -> Option<u32> {
    let e = end(nav, l);
    let v = net.lanes[l as usize].v1 as usize;
    let hits: Vec<u32> = net.vert.get(v).map_or(&[][..], |x| x.as_slice()).iter().copied().filter(|&o| start(nav, o) == e && o != l).collect();
    if hits.is_empty() {
        return None;
    }
    Some(hits[shared.index(hits.len() as u32) as usize])
}

/// F-74277: the lane after `l` - on the route F-52442 (node i strict, i + 1 strict, i loose, i + 1 loose, i the node
/// of `l` from `last`), else the ordinary pick.
pub fn route_next(route: &Route, net: &RailNet, nav: &NavSys, last: i32, l: u32, car: &mut Mwc, shared: &mut Mwc) -> Option<u32> {
    let i = route_index(route, net, last, Some(l));
    if i >= 0 {
        for (j, strict) in [(i, true), (i + 1, true), (i, false), (i + 1, false)] {
            if let Some(n) = node_next(route, net, nav, l, car, j, strict) {
                return Some(n);
            }
        }
    }
    ordinary_next(net, nav, l, shared)
}

/// F-21784: the destination tangent . (destination - p), z first, then y and x fused (the difference by the vector unit,
/// denormals flushed).
pub fn end_plane(route: &Route, p: Vec3) -> f32 {
    let (d, v) = (route.dest_dir, crate::tcp_rail::vsub3(route.dest_point, p));
    fma(d.x, v.x, fma(d.y, v.y, d.z * v.z))
}

/// F-48537: |destination - p|, x^2, then y and z fused (the difference by the vector unit, denormals flushed).
pub fn dest_dist(route: &Route, p: Vec3) -> f32 {
    let v = crate::tcp_rail::vsub3(route.dest_point, p);
    fma(v.z, v.z, fma(v.y, v.y, v.x * v.x)).sqrt()
}

/// The speed controller's entry 3 F-25114 for the cruising ghost at `p`: with flag 0x20 (the destination speed +252) and
/// the car before the destination's plane (F-21784 > 0), (the destination speed, the distance to the destination point
/// F-48537) at margin 0 and no reset bit; else none (the distance still FLT_MAX, the entry's sentinel, is none too).
pub fn dest_entry(route: &Route, p: Vec3, flags: u8, dest_speed: f32) -> Option<(f32, f32)> {
    if flags & FLAG_DEST_SPEED == 0 || !(end_plane(route, p) > 0.0) {
        return None;
    }
    let d = dest_dist(route, p);
    (d != f32::MAX).then_some((dest_speed, d))
}

/// F-44936: on the last node and past the destination's plane (0.5 m before it when stopping there: flag 0x20, the
/// destination speed and the car's speed 0).
pub fn route_end(route: &Route, last: i32, p: Vec3, flags: u8, dest_speed: f32, speed: f32) -> bool {
    let n = route.nodes.len() as i32;
    if n == 0 || last != n - 1 {
        return false;
    }
    let mut d = end_plane(route, p);
    if flags & FLAG_DEST_SPEED != 0 && dest_speed == 0.0 && speed == 0.0 {
        d -= 0.5;
    }
    d < 0.0
}

/// F-20872: a route, the current rail an ordinary lane (flags & 0x1800 clear) and no node holds it.
pub fn off_route(route: &Route, t: &Track, ordinary: bool) -> bool {
    !route.nodes.is_empty() && ordinary && t.cur < 0
}

/// F-67660: the lane's first place in the node's lanes is usable.
fn usable_in(node: &Node, lane: Option<u32>) -> bool {
    let Some(lane) = lane else { return false };
    node.lanes.iter().position(|&o| o == lane).is_some_and(|i| node.usable[i])
}

/// F-96498: a route lane change is wanted - a node for the current lane, its road (F-86691) faster than 16 mph
/// (F-33244: +148 <= 7.15264 is not), and neither the current lane nor the next one usable here, nor the next one
/// usable in the following node.
pub fn change_wanted(route: &Route, net: &RailNet, idx: i32, cur: Option<u32>, next: Option<u32>) -> bool {
    if idx < 0 {
        return false;
    }
    let Some(c) = cur else { return false };
    if let Some(r) = net.lanes[c as usize].rec.filter(|&r| net.recs[r as usize].flags & 2 != 0) {
        if net.recs[r as usize].speed <= f32::from_bits(0x40E4_E26D) {
            return false;
        }
    }
    let n = route.nodes.len() as i32;
    let node = &route.nodes[idx as usize];
    if usable_in(node, cur) || usable_in(node, next) {
        return false;
    }
    !(idx < n - 1 && usable_in(&route.nodes[idx as usize + 1], next))
}

/// F-36745 / F-67514's ramp: lo + u (hi - lo), u = clamp((v x 2.236936 - 10) / (80 - 10), 0, 1).
fn ramp(v: f32, lo: f32, hi: f32) -> f32 {
    let u = fms(v, f32::from_bits(0x400F_29F7), 10.0) / (80.0 - 10.0);
    let a = if u >= 0.0 { u } else { 0.0 };
    let b = if a - 1.0 >= 0.0 { 1.0 } else { a };
    fma(b, hi - lo, lo)
}

/// F-36745: the lane change's length, 15 .. 75 m by the speed (m/s).
pub fn change_len(v: f32) -> f32 {
    ramp(v, 15.0, 75.0)
}

/// F-67514: its second length, 4 .. 10.
pub fn change_w(v: f32) -> f32 {
    ramp(v, 4.0, 10.0)
}

/// F-85280: the knot span holding u (the first j with K[j] <= u < K[j + 1]).
fn span(c: &LaneCurve, u: f32) -> usize {
    let kn = &c.knots;
    let mut j = 0;
    while j + 2 < kn.len() && !(kn[j] <= u && u < kn[j + 1]) {
        j += 1;
    }
    j
}

/// F-94373: the derivative of the basis N(i, k) at u, from the lower order's pairs (F-75457) in the original's order.
fn dbasis(c: &LaneCurve, i: usize, k: usize, u: f32) -> f32 {
    if k == 1 {
        return 0.0;
    }
    let kn = &c.knots;
    let (f28, f30) = (u - kn[i], kn[i + k - 1] - kn[i]);
    let (f27, f29) = (kn[i + k] - u, kn[i + k] - kn[i + 1]);
    if f30 != 0.0 {
        let (n0, d0) = c.basis_pair(i, k - 1, u);
        let f11 = 1.0 / f30;
        if f29 != 0.0 {
            let (n1, d1) = c.basis_pair(i + 1, k - 1, u);
            let f10 = 1.0 / f29;
            let (f9, f8, f7) = (f11 * f28, f11 * n0, f10 * f27);
            fma(f7, d1, fnms(f10, n1, fma(f9, d0, f8)))
        } else {
            fma(f11 * f28, d0, f11 * n0)
        }
    } else if f29 != 0.0 {
        let (n1, d1) = c.basis_pair(i + 1, k - 1, u);
        let f12 = 1.0 / f29;
        fms(f12 * f27, d1, f12 * n1)
    } else {
        0.0
    }
}

/// Lane slot 11 (F-42559 -> F-75223): |dP/dt| at t - t clamped to 0 .. 1, u = t x TRange capped at TMax, the sum of
/// the degree + 1 control points x the basis derivative (the vector unit's multiply-add, not fused), the length (z^2,
/// then y and x fused) x TRange; averaged with `prev` when that is above 0.
pub fn lane_speed(c: &LaneCurve, t: f32, prev: f32) -> f32 {
    let t13 = if t >= 0.0 { t } else { 0.0 };
    let t = if t13 - 1.0 >= 0.0 { 1.0 } else { t13 };
    let prod = c.t_range * t;
    let u = if prod - c.t_max >= 0.0 { c.t_max } else { prod };
    let d = c.degree as usize;
    let start = span(c, u).saturating_sub(d).min(c.pts.len().saturating_sub(d + 1));
    let mut a = Vec3::ZERO;
    for i in start..=start + d {
        let w = dbasis(c, i, d + 1, u);
        let p = c.pts[i];
        a = Vec3::new(p.x * w + a.x, p.y * w + a.y, p.z * w + a.z);
    }
    let s = fma(a.x, a.x, fma(a.y, a.y, a.z * a.z)).sqrt() * c.t_range;
    if prev > 0.0 {
        (s + prev) * 0.5
    } else {
        s
    }
}

/// F-49677: the lane and phase `len` metres ahead of `cur` at t, a metre a step (lane slot 11, then slot 6 F-77217),
/// onto `next` at the end of `cur`; past the end of `next` (or of `cur` without one): no lane.
pub fn ahead(nav: &NavSys, cur: u32, next: Option<u32>, len: f32, t: f32) -> (Option<u32>, f32) {
    let (mut lane, mut t, mut s, mut rem) = (Some(cur), t, 0.0f32, len);
    if rem > 0.0 {
        while t < 1.0 {
            let l = lane.unwrap();
            let c = &nav.curves[l as usize];
            s = lane_speed(c, t, s);
            t = LaneCurve::advance(t, 1.0, s);
            if t >= 1.0 {
                if l == cur && next.is_some() {
                    lane = next;
                    t = 0.0;
                    s = 0.0;
                } else {
                    lane = None;
                }
            }
            rem -= 1.0;
            if !(rem > 0.0) {
                break;
            }
        }
    }
    (lane, t)
}

/// F-99155: the node's lane other than `lane` (usable only when asked) nearest to `lane`'s point at t (slot 7
/// F-99015) among those it falls inside (slot 14 F-24415: 0 < phase < 1), with that phase.
fn beside(nav: &NavSys, node: &Node, lane: u32, t: f32, usable: bool) -> Option<(u32, f32)> {
    let p = crate::railroute::point(&nav.curves[lane as usize], t);
    let mut best = f32::MAX;
    let mut out = None;
    for (i, &o) in node.lanes.iter().enumerate() {
        if o == lane || (usable && !node.usable[i]) {
            continue;
        }
        let (ph, d2, _) = crate::railroute::nearest(&nav.curves[o as usize], p, 4, 10);
        if !(ph > 0.0) || !(ph < 1.0) || !(d2 < best) {
            continue;
        }
        out = Some((o, ph));
        best = d2;
    }
    out
}

/// F-81401: `pin`'s lane projected onto a lane of its node (F-99155); `out` unchanged without a lane or a node, its
/// phase unchanged when nothing is found.
fn target(route: &Route, nav: &NavSys, pin: &Pin, usable: bool, mut out: Pin) -> Pin {
    let Some(l) = pin.lane else { return out };
    if pin.idx < 0 {
        return out;
    }
    let hit = beside(nav, &route.nodes[pin.idx as usize], l, pin.phase, usable);
    out.lane = hit.map(|h| h.0);
    if let Some((_, ph)) = hit {
        out.phase = ph;
    }
    out.idx = pin.idx;
    out
}

/// F-97539: `len` metres on from `a` - on its own lane when it is long enough (lane slot 20: +12, F-22677), else
/// through the route's lanes joined end to start (usable only when asked), at most three nodes; each node starts the
/// answer at its lane with phase 1.
fn walk_on(route: &Route, nav: &NavSys, a: &Pin, usable: bool, len: f32, mut out: Pin) -> Pin {
    let Some(mut l) = a.lane else { return out };
    let mut idx = a.idx;
    let ll = nav.lane_len(l as usize);
    let mut run = fnms(ll, a.phase, ll);
    if run > len {
        out.idx = idx;
        out.lane = Some(l);
        out.phase = (ll * a.phase + len) / ll;
        return out;
    }
    for _ in 0..3 {
        if idx >= route.nodes.len() as i32 {
            return out;
        }
        let node = &route.nodes[idx as usize];
        out = Pin { lane: Some(l), phase: 1.0, idx };
        let mut i = 0;
        while i < node.lanes.len() {
            let o = node.lanes[i];
            if o != l && (!usable || node.usable[i]) && end(nav, l) == start(nav, o) {
                l = o;
                let ol = nav.lane_len(o as usize);
                if ol + run > len {
                    return Pin { lane: Some(o), phase: (len - run) / ol, idx };
                }
                run = ol + run;
                i = 0;
                continue;
            }
            i += 1;
        }
        idx += 1;
    }
    out
}

/// F-24481 (behaviour +528, the riding states; the caller skips it while the context's bit 0x40 is set): with a
/// route and F-96498's yes, the lane beside the current one (F-81401) and the one beside the point `len` metres ahead
/// (F-49677, its node from `t.last`); without the latter, `len` metres on from the former (F-97539). The answer is the
/// lane-change rail's arguments (F-32118 is called with them).
pub fn lane_change(route: &Route, net: &RailNet, nav: &NavSys, t: &Track, cur: u32, next: Option<u32>, phase: f32, v: f32) -> Option<Change> {
    if route.nodes.is_empty() || !change_wanted(route, net, t.cur, Some(cur), next) {
        return None;
    }
    let (len, w) = (change_len(v), change_w(v));
    let here = Pin { lane: Some(cur), phase, idx: t.cur };
    let (l2, ph2) = ahead(nav, cur, next, len, phase);
    let there = Pin { lane: l2, phase: ph2, idx: route_index(route, net, t.last, l2) };
    let a = target(route, nav, &here, true, Pin::NONE);
    let mut b = target(route, nav, &there, true, Pin::NONE);
    if a.lane.is_none() && b.lane.is_none() {
        return None;
    }
    if b.lane.is_none() {
        b = walk_on(route, nav, &a, true, len, b);
    }
    let to = b.lane?;
    Some(Change { from: cur, to, t: phase, phase: b.phase, len, w })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::railroute;

    /// F-25114 run offline on the original's code (work/traffic-detail/e3, 6,000 cases: the flag byte, the destination
    /// speed, point and direction, the car's place before, on and past the plane, with zeros, -0, NaN, infinities, FLT_MAX
    /// and a denormal mixed in): the entry's active bit, distance (+4) and speed (+8) equal [`dest_entry`]'s to the bit;
    /// its margin (+12) is 0 and its reset bit (0x40) cleared every frame.
    #[test]
    fn dest_entry_matches_the_original() {
        let (mut n, mut on) = (0, 0);
        for l in include_str!("testdata/dest_entry_ref.txt").lines().filter(|l| !l.starts_with('#')) {
            let (a, b) = l.split_once(" | ").unwrap();
            let w: Vec<&str> = a.split_whitespace().collect();
            let o: Vec<&str> = b.split_whitespace().collect();
            let v = |k: usize| Vec3::new(fx(w[k]), fx(w[k + 1]), fx(w[k + 2]));
            let route = Route { dest_point: v(2), dest_dir: v(5), ..Default::default() };
            let got = dest_entry(&route, v(8), hx(w[0]) as u8, fx(w[1]));
            let b16 = hx(o[4]);
            assert_eq!(b16 & 0x80 != 0, got.is_some(), "active {l}");
            assert_eq!(b16 & 0x3F, hx(w[11]) & 0x3F, "low bits {l}");
            assert_eq!(b16 & 0x40, 0, "reset {l}");
            assert_eq!(hx(o[3]), 0, "margin {l}");
            let (sp, d) = got.map_or((0, 0), |(s, d)| (s.to_bits(), d.to_bits()));
            assert_eq!((hx(o[1]), hx(o[2])), (d, sp), "dist / speed {l}");
            n += 1;
            on += got.is_some() as usize;
        }
        assert_eq!(n, 6000);
        assert!(on > 1000);
    }

    fn load() -> Option<(NavSys, RailNet)> {
        let root = std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into()));
        let res = crate::assets::load_resource_file(&root.join("resources/city/sc/0x9960fcd8.r66")).ok()?;
        Some((NavSys::parse(&res).ok()?, RailNet::parse(&res)))
    }

    /// (a word 82xxxxxx is written h82xxxxxx in the testdata, so the registry's address scan does not take it for code)
    fn hx(s: &str) -> u32 {
        u32::from_str_radix(s.trim_start_matches('h'), 16).unwrap()
    }
    fn fx(s: &str) -> f32 {
        f32::from_bits(hx(s))
    }
    fn lane(s: &str) -> Option<u32> {
        let v: i64 = s.parse().unwrap();
        (v >= 0).then_some(v as u32)
    }

    /// [equal, total] per line kind S / E / L / X, the cases whose route did not rebuild, and the first differences.
    #[derive(Default, Debug)]
    struct Tally {
        s: [usize; 2],
        e: [usize; 2],
        l: [usize; 2],
        lc: [usize; 2],
        /// changes equal but for the phase on the target lane
        phase_only: usize,
        x: [usize; 2],
        skipped: usize,
        bad: Vec<String>,
    }

    /// The route of a C line: the search from its start to the original's snapped destination (railroute::search, bit
    /// equal to the original's), checked against the line's nodes, with the line's destination point and tangent.
    fn route_of(line: &str, net: &RailNet, nav: &NavSys) -> Option<Route> {
        let (head, _) = line.split_once(" | vc ").unwrap();
        let parts: Vec<&str> = head.split(" | ").collect();
        let a: Vec<&str> = parts[0].split(' ').collect();
        let b: Vec<&str> = parts[2].split(' ').collect();
        let (sl, t): (u32, f32) = (a[3].parse().unwrap(), fx(a[5]));
        let mut want: Vec<(i64, Vec<(i64, bool)>)> = Vec::new();
        for nd in parts.get(4).copied().unwrap_or("").split('[').skip(1) {
            let w: Vec<&str> = nd.trim_end_matches(|c| c == ']' || c == ' ').split(' ').collect();
            let rec = w[0].split_once('<').unwrap().0.parse().unwrap();
            want.push((rec, w[5..].iter().map(|x| (x.trim_end_matches('*').parse::<i64>().unwrap(), x.ends_with('*'))).collect()));
        }
        let mut r = Route { dest_point: Vec3::new(fx(b[9]), fx(b[10]), fx(b[11])), dest_dir: Vec3::new(fx(b[13]), fx(b[14]), fx(b[15])), ..Default::default() };
        if let Some(dl) = lane(b[5]) {
            r.dest_lane = Some(dl);
            r.dest_t = fx(b[7]);
            r.nodes = railroute::search(net, nav, sl, t, dl, r.dest_t).0.unwrap_or_default();
        }
        let got: Vec<(i64, Vec<(i64, bool)>)> = r.nodes.iter().map(|n| (n.rec as i64, n.lanes.iter().zip(&n.usable).map(|(&l, &u)| (l as i64, u)).collect())).collect();
        (got == want).then_some(r)
    }

    fn check(text: &str) -> Tally {
        let mut tl = Tally::default();
        let Some((nav, net)) = load() else { return tl };
        let mut route: Option<Route> = None;
        let (mut flags, mut ds) = (0u8, 0.0f32);
        let mut tr = Track::default();
        let (mut cur, mut next) = (None, None);
        let bad = |tl: &mut Tally, s: String| {
            if tl.bad.len() < 12 {
                tl.bad.push(s);
            }
        };
        for line in text.lines() {
            let w: Vec<&str> = line.split(' ').collect();
            match w[0] {
                "C" => {
                    route = route_of(line, &net, &nav);
                    tl.skipped += route.is_none() as usize;
                    tr = Track::default();
                }
                "R" => {
                    flags = hx(w[3]) as u8;
                    ds = fx(w[5]);
                }
                _ => {}
            }
            let Some(r) = route.as_ref() else { continue };
            match w[0] {
                "S" => {
                    cur = lane(w[4]);
                    let (i128, i132): (i32, i32) = (w[6].parse().unwrap(), w[8].parse().unwrap());
                    next = lane(w[10]);
                    let mut car = Mwc { lo: hx(w[12]), carry: hx(w[13]) };
                    let mut sh = Mwc { lo: hx(w[18]), carry: hx(w[19]) };
                    track(r, &net, &mut tr, cur);
                    let n = route_next(r, &net, &nav, tr.last, cur.unwrap(), &mut car, &mut sh);
                    let ok = tr.cur == i128 && tr.last == i132 && n == next && (car.lo, car.carry) == (hx(w[15]), hx(w[16])) && (sh.lo, sh.carry) == (hx(w[21]), hx(w[22]));
                    tl.s[0] += ok as usize;
                    tl.s[1] += 1;
                    if !ok {
                        bad(&mut tl, format!("{line}\n  port i128 {} i132 {} next {n:?} car {:08x} {:08x} sh {:08x} {:08x}", tr.cur, tr.last, car.lo, car.carry, sh.lo, sh.carry));
                    }
                    // the original's state goes on, whatever the port said
                    tr = Track { cur: i128, last: i132 };
                }
                "E" => {
                    let p = Vec3::new(fx(w[5]), fx(w[6]), fx(w[7]));
                    let v = fx(w[9]);
                    let e = route_end(r, tr.last, p, flags, ds, v);
                    let (pl, di) = (end_plane(r, p), dest_dist(r, p));
                    let ok = e == (w[11] == "1") && pl.to_bits() == hx(w[13]) && di.to_bits() == hx(w[15]);
                    tl.e[0] += ok as usize;
                    tl.e[1] += 1;
                    if !ok {
                        bad(&mut tl, format!("{line}\n  port end {e} plane {:08x} dist {:08x}", pl.to_bits(), di.to_bits()));
                    }
                }
                "X" => {
                    let ok = off_route(r, &tr, true);
                    tl.x[0] += ok as usize;
                    tl.x[1] += 1;
                }
                "L" => {
                    let (t, v) = (fx(w[4]), fx(w[6]));
                    let c = cur.unwrap();
                    let gate = change_wanted(r, &net, tr.cur, cur, next);
                    let ch = lane_change(r, &net, &nav, &tr, c, next, t, v);
                    let want = (w[10] == "1").then(|| Change { from: lane(w[12]).unwrap(), to: lane(w[14]).unwrap(), t: fx(w[18]), phase: fx(w[19]), len: fx(w[20]), w: fx(w[21]) });
                    let bits = |c: &Option<Change>| c.map(|c| (c.from, c.to, c.t.to_bits(), c.phase.to_bits(), c.len.to_bits(), c.w.to_bits()));
                    let ok = gate == (w[8] == "1") && bits(&ch) == bits(&want) && (want.is_none() || w[16] == "0");
                    tl.l[0] += ok as usize;
                    tl.l[1] += 1;
                    if want.is_some() {
                        tl.lc[0] += ok as usize;
                        tl.lc[1] += 1;
                    }
                    let lanes = |c: &Option<Change>| c.map(|c| (c.from, c.to, c.t.to_bits(), c.len.to_bits(), c.w.to_bits()));
                    if !ok && gate == (w[8] == "1") && want.is_some() && lanes(&ch) == lanes(&want) {
                        tl.phase_only += 1;
                    }
                    if !ok {
                        bad(&mut tl, format!("{line}\n  port gate {gate} change {ch:?}"));
                    }
                }
                _ => {}
            }
        }
        tl
    }

    fn report(name: &str, tl: &Tally) {
        println!(
            "ghostroute {name}: S {} / {}, E {} / {}, L {} / {} (changes {} / {}, phase only {}), X {} / {}, routes not rebuilt {}",
            tl.s[0], tl.s[1], tl.e[0], tl.e[1], tl.l[0], tl.l[1], tl.lc[0], tl.lc[1], tl.phase_only, tl.x[0], tl.x[1], tl.skipped
        );
        for b in &tl.bad {
            println!("{b}");
        }
    }

    /// F-74277 / F-52442 / F-20639 (next lane and both random numbers), F-18846 / F-74061 (indices), F-44936 /
    /// F-21784 / F-48537 (route end, plane, distance), F-96498 / F-24481 (gate and lane change: from, to, t, phase,
    /// length, width), F-20872 (off route) against the original's code run offline on the city railnet
    /// (work/racerai/ghost/h, 120 cases, seed 1).
    #[test]
    fn ghost_route_matches_the_original() {
        let tl = check(include_str!("testdata/ghostroute_120.txt"));
        if tl.s[1] == 0 {
            return;
        }
        report("120", &tl);
        assert_eq!((tl.s[0], tl.e[0], tl.l[0], tl.x[0]), (tl.s[1], tl.e[1], tl.l[1], tl.x[1]));
    }

    /// The same on the 2,000 cases of work/racerai/ghost/h/out_2000.txt (seed 2) when it is there.
    #[test]
    fn ghost_route_matches_the_original_2000() {
        let p = std::env::var("MCLA_GHOSTROUTE_REF").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../work/racerai/ghost/h/out_2000.txt"));
        let Ok(text) = std::fs::read_to_string(p) else { return };
        let tl = check(&text);
        if tl.s[1] == 0 {
            return;
        }
        report("2000", &tl);
        assert_eq!((tl.s[0], tl.e[0], tl.l[0], tl.x[0]), (tl.s[1], tl.e[1], tl.l[1], tl.x[1]));
    }
}
