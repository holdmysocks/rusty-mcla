//! The cruising brain's rail helpers (docs/reference/ai/brain_cruising.md section 3): the nearest rail with a heading
//! (F-70158), the walk along the rails by a distance (F-38744, the next rail picked by F-20639 with the shared random
//! numbers), the brain's offset in a rail's frame (F-14929), the rail pose (F-12468), the placement on the nearest rail
//! with the ground probe (F-97236, F-29307), the two rail goals (F-90991) and DriveToNearestRail (F-12584); the rail
//! speed is lane slot 17 (F-27898). Built on the raw navsys: `RailNet` (railroute.rs) for the lanes' vertices and owners,
//! `NavSys` for the lane curves and the lane quadtree.
//!
//! The world is left to the caller: the ground probe (the shape test F-20855 from 5 m above the point to 5 m below) and
//! the nav-mesh test of F-12584 (F-72689) are callbacks. The racing side (the reset F-69438, AppendGoal F-84673,
//! MaxSpeed F-39608, FinalHeading F-27882, FinalSpeed F-59324) comes back as data, `RailGoals`.
//!
//! Arithmetic as the original's: the scalar unit's multiply-adds fused (one rounding in double, then single), the
//! vector unit's not; its three-term sums (0 + z z') + (y y' + x x').

use crate::physics::exact::fma;
use crate::railroute::{vnorm, RailNet};
use crate::vehicles::parked::Mwc;
use crate::vehicles::traffic::rail_speed;
use glam::Vec3;
use mcla_formats::rage::navsys::NavSys;
use mcla_formats::rsc::Resource;

/// The radii the nearest-rail searches try in turn (F-70158, F-97236).
pub const RADII: [f32; 5] = [5.0, 10.0, 20.0, 40.0, 80.0];
/// The lane quadtree's split test (F-21575: the point is past the split by more than this).
const SPLIT_EPS: f32 = f32::from_bits(0xB586_37BD);
/// F-21575's child order per quadrant (the point's side of the x split x 2 + of the z split).
const CHILD_ORDER: [usize; 16] = [0, 1, 2, 3, 1, 3, 0, 2, 2, 0, 3, 1, 3, 2, 1, 0];
/// F-59579 keeps at most 64 lane answers.
const MAX_HITS: usize = 64;
/// F-41334's least squared length (the callers' 1e-5).
const NORM_EPS: f32 = f32::from_bits(0x3727_C5AC);

/// A lane's own fields the rail code reads.
#[derive(Clone, Copy, Debug, Default)]
struct RailLane {
    /// +4
    flags: u32,
    /// +0x0c: the stored length (lane slot 20, F-22677)
    len: f32,
    /// +0x10 / +0x20: its box
    min: Vec3,
    max: Vec3,
    /// its first link in `NavSys::links` (for the rail speed)
    link: Option<usize>,
}

/// The rails: the navsys tables and the lanes' fields.
pub struct Rails<'a> {
    pub net: &'a RailNet,
    pub nav: &'a NavSys,
    lanes: Vec<RailLane>,
    /// (tests) the original's lane answers replayed in place of the port's curve search
    #[cfg(test)]
    replay: std::cell::RefCell<Option<Replay>>,
}

/// (tests) the lane slot 3 answers the original gave, in call order (the valid ones), and the calls seen.
#[cfg(test)]
#[derive(Default)]
struct Replay {
    calls: usize,
    answers: std::collections::VecDeque<(f32, RailHit)>,
}

/// A lane's answer to a point (F-15621's 48-byte record: point, tangent, lane, squared distance, phase).
#[derive(Clone, Copy, Debug)]
pub struct RailHit {
    pub lane: u32,
    pub t: f32,
    pub d2: f32,
    pub point: Vec3,
    pub dir: Vec3,
}

/// The brain's rail and its phase (+1700 / +1704).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RailPos {
    pub lane: Option<u32>,
    pub t: f32,
}

/// A matrix as the brain writes it: rows a, b, c (c = backwards) and the position.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frame {
    pub a: Vec3,
    pub b: Vec3,
    pub c: Vec3,
    pub pos: Vec3,
}

/// What F-90991 hands the racing brain after its reset: the goals (point, radius) in order, MaxSpeed, FinalHeading
/// (none: not set) and FinalSpeed; and the brain's closest distance to the last goal (+1696).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RailGoals {
    pub goals: Vec<(Vec3, f32)>,
    pub max_speed: f32,
    pub final_heading: Option<Vec3>,
    pub final_speed: f32,
    pub closest: Option<f32>,
}

/// DriveToNearestRail's outcome (F-12584): the state it leaves and the racing goals it set.
#[derive(Clone, Debug, PartialEq)]
pub enum Drive {
    /// state 1: on the two rail goals
    Rail(RailGoals),
    /// state 2: to waypoint 0 (one goal of 5 m, MaxSpeed and FinalSpeed -1)
    Waypoint(RailGoals),
    /// state 4: teleport
    Teleport,
}

/// F-41334: `v` normalised when its squared length (y^2, then x, then z fused) is above `eps`, the reciprocal square root
/// refined twice in double; else `fallback`. Also whether it was normalised.
pub fn normalize_or(v: Vec3, eps: f32, fallback: Vec3) -> (Vec3, bool) {
    let f0 = fma(v.z, v.z, fma(v.x, v.x, v.y * v.y));
    if !(f0 > eps) {
        return (fallback, false);
    }
    let x = f0 as f64;
    let y0 = (1.0 / x.sqrt()) as f32 as f64;
    let y1 = (-((y0 * y0).mul_add(x, -1.0))).mul_add(y0 * 0.5, y0);
    let y2 = (-((y1 * y1).mul_add(x, -1.0))).mul_add(y1 * 0.5, y1);
    let s = if -f0 >= 0.0 { 0.0 } else { y2 as f32 };
    (v * s, true)
}

/// The vector unit's crossed rows as the brain's code writes them: -(p.yzx q.zxy - p.zxy q.yzx), not fused.
fn ncross(p: Vec3, q: Vec3) -> Vec3 {
    Vec3::new(-(p.y * q.z - p.z * q.y), -(p.z * q.x - p.x * q.z), -(p.x * q.y - p.y * q.x))
}

/// F-14929: `p` moved by the brain's offset (+1632) in the frame of a rail running along `dir`: c = -dir normalised
/// (fallback (0, 0, -1)), up = +y, a = up x c; each coordinate (p + oz c) + (oy up + ox a), the vector unit's sums.
pub fn offset_point(off: Vec3, p: Vec3, dir: Vec3) -> Vec3 {
    let (c, _) = normalize_or(Vec3::ZERO - dir, NORM_EPS, Vec3::ZERO - Vec3::Z);
    let u = Vec3::Y;
    let a = ncross(c, u);
    let row = |pk: f32, ck: f32, uk: f32, ak: f32| (pk + off.z * ck) + (off.y * uk + off.x * ak);
    Vec3::new(row(p.x, c.x, u.x, a.x), row(p.y, c.y, u.y, a.y), row(p.z, c.z, u.z, a.z))
}

/// F-29307: the ground under a frame. The probe (the world's shape test, from the position 5 m up to 5 m down) gives
/// the hit point and normal: the position goes there and b = the normal, else b = +y; then b, a = c x b and c = b x a
/// normalised (the vector unit's way). Whether it hit.
pub fn ground(m: &mut Frame, probe: &mut dyn FnMut(Vec3, Vec3) -> Option<(Vec3, Vec3)>) -> bool {
    let up = Vec3::new(0.0, 5.0, 0.0);
    let hit = probe(m.pos + up, m.pos - up);
    let n = match hit {
        Some((p, n)) => {
            m.pos = p;
            n
        }
        None => Vec3::Y,
    };
    m.b = vnorm(n);
    m.a = vnorm(ncross(m.c, m.b));
    m.c = vnorm(ncross(m.b, m.a));
    hit.is_some()
}

impl<'a> Rails<'a> {
    /// The lanes' fields from the navsys resource (lanes in file order, as `RailNet` and `NavSys::curves`).
    pub fn new(res: &Resource, nav: &'a NavSys, net: &'a RailNet) -> Rails<'a> {
        let d = &res.data;
        let be = |o: usize| u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]);
        let fl = |o: usize| f32::from_bits(be(o));
        let v3 = |o: usize| Vec3::new(fl(o), fl(o + 4), fl(o + 8));
        let mut lanes = Vec::new();
        let mut o = 0usize;
        while o + 0x30 <= res.virtual_size {
            match u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) {
                0x0047_9f0c => {
                    lanes.push(RailLane { flags: be(o + 4), len: fl(o + 0x0c), min: v3(o + 0x10), max: v3(o + 0x20), link: None });
                    o += 0x70;
                }
                0x0047_a13c => o += 0xb0,
                0x0047_a0c4 => o += 0xc0,
                _ => o += 4,
            }
        }
        for (i, l) in nav.links.iter().enumerate() {
            if let Some(x) = lanes.get_mut(l.lane) {
                x.link.get_or_insert(i);
            }
        }
        Rails {
            net,
            nav,
            lanes,
            #[cfg(test)]
            replay: Default::default(),
        }
    }

    /// Lane slot 20 (F-22677): the stored length.
    pub fn length(&self, lane: u32) -> f32 {
        self.lanes[lane as usize].len
    }

    /// Lane slot 17 (F-27898): the rail speed, m/s.
    pub fn speed(&self, lane: u32) -> f32 {
        self.lanes[lane as usize].link.map_or(0.0, |k| rail_speed(self.nav, k).0)
    }

    /// Lane slot 3 (F-15621): the box test (slot 5 F-56243: the box's distance (0 + z^2) + (y^2 + x^2) under r^2), then
    /// the nearest curve point (F-53741: 8 chords from 450 m of stored length, else 4; 10 halvings), valid under r^2.
    fn lane_near(&self, lane: usize, p: Vec3, r: f32) -> Option<RailHit> {
        #[cfg(test)]
        if let Some(rp) = self.replay.borrow_mut().as_mut() {
            rp.calls += 1;
            if rp.answers.front().is_some_and(|(rr, h)| h.lane as usize == lane && rr.to_bits() == r.to_bits()) {
                return rp.answers.pop_front().map(|(_, h)| h);
            }
            return None;
        }
        let l = &self.lanes[lane];
        let o = (l.min - p).max(p - l.max).max(Vec3::ZERO);
        let (ox, oy, oz) = (o.x * o.x, o.y * o.y, o.z * o.z);
        let r2 = r * r;
        if !(r2 > (0.0 + oz) + (oy + ox)) {
            return None;
        }
        let c = &self.nav.curves[lane];
        let n = if l.len < 450.0 { 4 } else { 8 };
        let (t, d2, point) = crate::railroute::nearest(c, p, n, 10);
        if !(d2 < r2) {
            return None;
        }
        Some(RailHit { lane: lane as u32, t, d2, point, dir: crate::railroute::deriv(c, t) })
    }

    /// F-21575: the lane quadtree from `node`: a node whose x / z box comes within r (F-80185) gives its children in
    /// the order of the point's quadrant, then its own lanes with flags & 1; at most `cap` answers.
    fn walk(&self, node: usize, p: Vec3, r: f32, cap: usize, out: &mut Vec<RailHit>) {
        let q = &self.nav.quad[node];
        let mut d = 0.0f32;
        if p.x < q.min[0] {
            let e = p.x - q.min[0];
            d = e * e;
        } else if p.x > q.max[0] {
            let e = p.x - q.max[0];
            d = e * e;
        }
        if p.z < q.min[1] {
            let e = p.z - q.min[1];
            d = fma(e, e, d);
        } else if p.z > q.max[1] {
            let e = p.z - q.max[1];
            d = fma(e, e, d);
        }
        if d > r * r {
            return;
        }
        let quadrant = if p.x - q.split[0] > SPLIT_EPS { 2 } else { 0 } | if p.z - q.split[1] > SPLIT_EPS { 1 } else { 0 };
        let start = out.len();
        for k in 0..4 {
            let got = out.len() - start;
            if got >= cap {
                break;
            }
            if let Some(ch) = q.kids[CHILD_ORDER[quadrant * 4 + k]] {
                self.walk(ch, p, r, cap - got, out);
            }
        }
        for &l in &q.lanes {
            if out.len() - start >= cap {
                break;
            }
            if self.lanes[l].flags & 1 == 0 {
                continue;
            }
            if let Some(h) = self.lane_near(l, p, r) {
                out.push(h);
            }
        }
    }

    /// F-59579: the lane nearest to `p` within `r` (least squared distance, the first of equals); with a heading only
    /// the answers whose tangent T has sign(T.dir) (T.dir)^2 >= |T|^2 / 2 (within 45 degrees, the same way).
    pub fn snap(&self, p: Vec3, r: f32, dir: Option<Vec3>) -> Option<RailHit> {
        let mut hits = Vec::new();
        if !self.nav.quad.is_empty() {
            self.walk(0, p, r, MAX_HITS, &mut hits);
        }
        let mut best: Option<RailHit> = None;
        let mut least = f32::MAX;
        for h in hits {
            if let Some(v) = dir {
                let t = h.dir;
                let dot = fma(v.x, t.x, fma(v.z, t.z, v.y * t.y));
                let len2 = fma(t.y, t.y, fma(t.x, t.x, t.z * t.z));
                let sq = dot * dot;
                let half = len2 * 0.5;
                let signed = if dot >= 0.0 { sq } else { -sq };
                if signed < half {
                    continue;
                }
            }
            if h.d2 < least {
                least = h.d2;
                best = Some(h);
            }
        }
        best
    }

    /// F-20639: the next rail after `lane`: the lanes of its last vertex, other than itself, that start at its last
    /// control point (xyz equal) and pass the mask (0x80 clear: no owner flag 0x10; 0x40 clear: no 0x20; 0x20 clear: the
    /// owner has 0x10 or 0x20); one of them by the shared random numbers ((lo & 0x7fffffff) % n). F-38744's 0x20 takes
    /// the lanes of owners without 0x10 / 0x20, its 0xe0 any.
    pub fn next_rail(&self, lane: u32, mask: u8, rng: &mut Mwc) -> Option<u32> {
        let ln = self.net.lanes.get(lane as usize)?;
        let pts = &self.nav.curves[lane as usize].pts;
        let end = *pts.last()?;
        let mut cands = Vec::new();
        for &o in self.net.vert.get(ln.v1 as usize)? {
            if self.nav.curves[o as usize].pts.first() != Some(&end) || o == lane {
                continue;
            }
            let f = self.net.lanes[o as usize].owner_flags;
            if mask & 0x80 == 0 && f & 0x10 != 0 {
                continue;
            }
            if mask & 0x40 == 0 && f & 0x20 != 0 {
                continue;
            }
            if mask & 0x20 == 0 && f & 0x30 == 0 {
                continue;
            }
            cands.push(o);
        }
        if cands.is_empty() {
            return None;
        }
        Some(cands[rng.index(cands.len() as u32) as usize])
    }

    /// F-70158: the nearest rail ahead of `dir` around `pos` (radii 5 .. 80 m) into `rp` (no rail: none, t 0); the
    /// distance to it.
    pub fn nearest(&self, rp: &mut RailPos, pos: Vec3, dir: Vec3) -> Option<f32> {
        for r in RADII {
            let h = self.snap(pos, r, Some(dir));
            rp.lane = h.map(|h| h.lane);
            rp.t = h.map_or(0.0, |h| h.t);
            if let Some(h) = h {
                return Some(h.d2.sqrt());
            }
        }
        None
    }

    /// F-38744: `rp` moved `dist` metres on along the rails (the stored lengths): past a rail's end onto the next
    /// (F-20639 with mask 0x20, else 0xe0), t from 0. False when no rail follows (then no rail, t 0).
    pub fn advance(&self, rp: &mut RailPos, dist: f32, rng: &mut Mwc) -> bool {
        let Some(mut lane) = rp.lane else { return false };
        let mut d = dist;
        let mut rest = self.length(lane) * (1.0 - rp.t);
        while d > rest {
            rp.t = 0.0;
            d -= rest;
            let next = self.next_rail(lane, 0x20, rng).or_else(|| self.next_rail(lane, 0xE0, rng));
            rp.lane = next;
            let Some(n) = next else { return false };
            lane = n;
            rest = self.length(lane);
        }
        rp.t = d / self.length(lane) + rp.t;
        true
    }

    /// F-12468: the pose at the brain's rail: the point (lane slot 8) moved by the offset in the frame of the normalised
    /// tangent (fallback +z); its position and c = -tangent.
    pub fn pose(&self, rp: &RailPos, off: Vec3) -> Option<(Vec3, Vec3)> {
        let lane = rp.lane?;
        let (p, d) = self.nav.curves[lane as usize].eval_pair(rp.t);
        let (t, _) = normalize_or(d, NORM_EPS, Vec3::Z);
        Some((offset_point(off, p, t), Vec3::ZERO - t))
    }

    /// F-97236: the car placed on the nearest rail to `point` (any heading, radii 5 .. 80 m): the rail point moved by
    /// the offset, c = -tangent, on the ground (F-29307); a radius whose probe misses gives the next. The rail into `rp`.
    pub fn place(&self, rp: &mut RailPos, point: Vec3, off: Vec3, probe: &mut dyn FnMut(Vec3, Vec3) -> Option<(Vec3, Vec3)>) -> Option<Frame> {
        for r in RADII {
            let h = self.snap(point, r, None);
            rp.lane = h.map(|h| h.lane);
            rp.t = h.map_or(0.0, |h| h.t);
            let Some(h) = h else { continue };
            let (t, _) = normalize_or(h.dir, NORM_EPS, Vec3::Z);
            let mut m = Frame { pos: offset_point(off, h.point, t), c: Vec3::ZERO - t, ..Default::default() };
            if ground(&mut m, probe) {
                return Some(m);
            }
        }
        None
    }

    /// F-90991: the two goals on the rail ahead of the car (position `pos`, matrix row c `back`; `speed` the physical
    /// car's speed, none without one): the nearest rail with the car's heading, a lookahead of max(max(20, 2 x the
    /// distance), (v^2 - v_rail^2) x 0.025) to the first goal (radius 5), 20 m more to the second (radius 2); MaxSpeed and
    /// FinalSpeed the second's rail speed, FinalHeading its tangent; the distance to the second. None: no goals (the
    /// rail in `rp` is then where the walk stopped).
    pub fn goals(&self, rp: &mut RailPos, pos: Vec3, back: Vec3, speed: Option<f32>, off: Vec3, rng: &mut Mwc) -> Option<RailGoals> {
        let fwd = Vec3::ZERO - back;
        let dist = self.nearest(rp, pos, fwd)?;
        let b = match speed {
            Some(s) => {
                let v = self.speed(rp.lane?);
                fma(s, s, -(v * v)) * 0.025
            }
            None => 0.0,
        };
        let twice = dist * 2.0;
        let near = if 20.0 - twice >= 0.0 { 20.0 } else { twice };
        let look = if near - b >= 0.0 { near } else { b };
        if !self.advance(rp, look, rng) {
            return None;
        }
        let (p1, _) = self.pose(rp, off)?;
        if !self.advance(rp, 20.0, rng) {
            return None;
        }
        let (p2, c2) = self.pose(rp, off)?;
        let v = self.speed(rp.lane?);
        let d = pos - p2;
        let closest = fma(d.y, d.y, fma(d.z, d.z, d.x * d.x)).sqrt();
        Some(RailGoals { goals: vec![(p1, 5.0), (p2, 2.0)], max_speed: v, final_heading: Some(Vec3::ZERO - c2), final_speed: v, closest: Some(closest) })
    }

    /// F-12584 DriveToNearestRail, after its remove-from-traffic (F-12926, the caller's): needs the ambient system and
    /// the car on the nav mesh (`on_mesh`, F-72689), else teleport; the rail goals (state 1); else, with a current
    /// waypoint (`waypoint0`: the list's first, when the index is >= 0), a goal of 5 m on it (state 2); else teleport.
    #[allow(clippy::too_many_arguments)]
    pub fn drive_to_nearest_rail(&self, rp: &mut RailPos, pos: Vec3, back: Vec3, speed: Option<f32>, off: Vec3, waypoint0: Option<Vec3>, ambient: bool, on_mesh: impl FnOnce(Vec3) -> bool, rng: &mut Mwc) -> Drive {
        if !ambient || !on_mesh(pos) {
            return Drive::Teleport;
        }
        if let Some(g) = self.goals(rp, pos, back, speed, off, rng) {
            return Drive::Rail(g);
        }
        match waypoint0 {
            Some(w) => Drive::Waypoint(RailGoals { goals: vec![(w, 5.0)], max_speed: -1.0, final_heading: None, final_speed: -1.0, closest: None }),
            None => Drive::Teleport,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(s: &str) -> f32 {
        f32::from_bits(u32::from_str_radix(s, 16).unwrap())
    }
    fn v(w: &[&str]) -> Vec3 {
        Vec3::new(f(w[0]), f(w[1]), f(w[2]))
    }
    fn bits(a: Vec3) -> [u32; 3] {
        a.to_array().map(f32::to_bits)
    }
    fn lane(s: &str) -> Option<u32> {
        let i: i64 = s.parse().unwrap();
        (i >= 0).then_some(i as u32)
    }
    fn rng(lo: &str, c: &str) -> Mwc {
        Mwc { lo: u32::from_str_radix(lo, 16).unwrap(), carry: u32::from_str_radix(c, 16).unwrap() }
    }

    fn load() -> Option<(Resource, NavSys, RailNet)> {
        let root = std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into()));
        let res = crate::assets::load_resource_file(&root.join("resources/city/sc/0x9960fcd8.r66")).ok()?;
        let nav = NavSys::parse(&res).ok()?;
        let net = RailNet::parse(&res);
        Some((res, nav, net))
    }

    /// The recorded goals of a G / D line: reset count, goals, max, final, heading.
    fn rec(w: &[&str]) -> (u32, Vec<(Vec3, f32)>, u32, u32, Option<[u32; 3]>) {
        let i = w.iter().position(|&x| x == "reset").unwrap();
        let reset = w[i + 1].parse().unwrap();
        let n: usize = w[i + 3].parse().unwrap();
        let goals = (0..n).map(|k| (v(&w[i + 4 + 4 * k..]), f(w[i + 7 + 4 * k]))).collect();
        let j = i + 4 + 4 * n;
        let head = (w[j + 5] == "1").then(|| [w[j + 6], w[j + 7], w[j + 8]].map(|s| u32::from_str_radix(s, 16).unwrap()));
        (reset, goals, u32::from_str_radix(w[j + 1], 16).unwrap(), u32::from_str_radix(w[j + 3], 16).unwrap(), head)
    }

    fn goals_bits(g: &RailGoals) -> (Vec<([u32; 3], u32)>, u32, u32, Option<[u32; 3]>) {
        (g.goals.iter().map(|(p, r)| (bits(*p), r.to_bits())).collect(), g.max_speed.to_bits(), g.final_speed.to_bits(), g.final_heading.map(bits))
    }

    /// Per tag: [equal, total]; and the first differing lines.
    fn check(text: &str) -> (std::collections::BTreeMap<char, [usize; 2]>, Vec<String>) {
        let mut out = std::collections::BTreeMap::new();
        let mut bad = Vec::new();
        let Some((res, nav, net)) = load() else { return (out, bad) };
        let rails = Rails::new(&res, &nav, &net);
        for line in text.lines() {
            let Some(tag) = line.chars().next().filter(|c| "SNAOPGD".contains(*c)) else { continue };
            let (head, tail) = line.split_once(" | ").unwrap_or((line, ""));
            let h: Vec<&str> = head.split(' ').collect();
            let t: Vec<&str> = tail.split(' ').collect();
            let answers = t.iter().position(|&x| x == "lanes").map(|k| {
                let total: usize = t[k + 1].parse().unwrap();
                let n: usize = t[k + 2].parse().unwrap();
                let hits = (0..n).map(|i| {
                    let w = &t[k + 3 + 10 * i..];
                    (f(w[1]), RailHit { lane: w[0].parse().unwrap(), t: f(w[2]), d2: f(w[3]), point: v(&w[4..]), dir: v(&w[7..]) })
                }).collect::<std::collections::VecDeque<_>>();
                (total, hits)
            });
            for mode in 0..2 {
            if mode == 1 {
                let Some((_, hits)) = &answers else { continue };
                *rails.replay.borrow_mut() = Some(Replay { calls: 0, answers: hits.clone() });
            }
            let ok = match tag {
                'S' => rails.speed(h[1].parse().unwrap()).to_bits() == u32::from_str_radix(h[2], 16).unwrap(),
                'N' => {
                    let mut rp = RailPos::default();
                    let d = rails.nearest(&mut rp, v(&h[2..]), v(&h[5..]));
                    let want = t[0] == "1";
                    d.is_some() == want && rp.lane == lane(t[1]) && rp.t.to_bits() == f(t[2]).to_bits() && (!want || d.unwrap().to_bits() == f(t[3]).to_bits())
                }
                'A' => {
                    let mut rp = RailPos { lane: lane(h[2]), t: f(h[3]) };
                    let mut g = rng(h[5], h[6]);
                    let r = rails.advance(&mut rp, f(h[4]), &mut g);
                    r == (t[0] == "1") && rp.lane == lane(t[1]) && rp.t.to_bits() == f(t[2]).to_bits() && (g.lo, g.carry) == (rng(t[4], t[5]).lo, rng(t[4], t[5]).carry)
                }
                'O' => bits(offset_point(v(&h[2..]), v(&h[5..]), v(&h[8..]))) == bits(v(&t[0..])),
                'P' => {
                    let k = t.iter().position(|&x| x == "probes").unwrap();
                    let n: usize = t[k + 1].parse().unwrap();
                    let rec: Vec<(bool, Vec3, Vec3, Vec3, Vec3)> = (0..n).map(|i| {
                        let w = &t[k + 2 + 13 * i..];
                        (w[0] == "1", v(&w[1..]), v(&w[4..]), v(&w[7..]), v(&w[10..]))
                    }).collect();
                    let mut i = 0;
                    let mut same_seg = true;
                    let mut probe = |a: Vec3, b: Vec3| {
                        let r = rec.get(i).copied();
                        i += 1;
                        let Some((hit, s, e, p, nn)) = r else {
                            same_seg = false;
                            return None;
                        };
                        same_seg &= bits(a) == bits(s) && bits(b) == bits(e);
                        hit.then_some((p, nn))
                    };
                    let mut rp = RailPos::default();
                    let m = rails.place(&mut rp, v(&h[2..]), v(&h[5..]), &mut probe);
                    let want = t[0] == "1";
                    let rows = [v(&t[3..]), v(&t[6..]), v(&t[9..]), v(&t[12..])];
                    same_seg && i == n && m.is_some() == want && rp.lane == lane(t[1]) && rp.t.to_bits() == f(t[2]).to_bits()
                        && m.map_or(true, |m| [m.a, m.b, m.c, m.pos].map(bits) == rows.map(bits))
                }
                'G' | 'D' => {
                    let o = if tag == 'D' { 7 } else { 2 };
                    let pos = v(&h[o..]);
                    let back = v(&h[o + 3..]);
                    let off = v(&h[o + 6..]);
                    let speed = (h[o + 9] == "1").then(|| f(h[o + 10]));
                    let mut g = rng(h[o + 11], h[o + 12]);
                    let mut rp = RailPos::default();
                    let (r_reset, r_goals, r_max, r_final, r_head) = rec(&t);
                    let want_rng = rng(t[6], t[7]);
                    let mut good;
                    if tag == 'G' {
                        let got = rails.goals(&mut rp, pos, back, speed, off, &mut g);
                        good = got.is_some() == (t[0] == "1");
                        if let Some(gg) = &got {
                            good &= r_reset == 1 && goals_bits(gg) == (r_goals.iter().map(|(p, r)| (bits(*p), r.to_bits())).collect(), r_max, r_final, r_head) && gg.closest.map(f32::to_bits) == Some(f(t[3]).to_bits());
                        } else {
                            good &= r_reset == 0 && r_goals.is_empty();
                        }
                    } else {
                        let on = h[2] == "1";
                        let idx: i64 = h[3].parse().unwrap();
                        let wp = (idx >= 0).then(|| v(&h[4..]));
                        let got = rails.drive_to_nearest_rail(&mut rp, pos, back, speed, off, wp, true, |_| on, &mut g);
                        let state: i64 = t[4].parse().unwrap();
                        good = match &got {
                            Drive::Rail(gg) => state == 1 && r_reset == 1 && goals_bits(gg) == (r_goals.iter().map(|(p, r)| (bits(*p), r.to_bits())).collect(), r_max, r_final, r_head) && gg.closest.map(f32::to_bits) == Some(f(t[3]).to_bits()),
                            Drive::Waypoint(gg) => state == 2 && r_reset == 1 && goals_bits(gg) == (r_goals.iter().map(|(p, r)| (bits(*p), r.to_bits())).collect(), r_max, r_final, r_head),
                            Drive::Teleport => state == 4 && r_reset == 0 && r_goals.is_empty(),
                        };
                    }
                    good && rp.lane == lane(t[1]) && rp.t.to_bits() == f(t[2]).to_bits() && (g.lo, g.carry) == (want_rng.lo, want_rng.carry)
                }
                _ => unreachable!(),
            };
            let ok = ok && match (mode, rails.replay.borrow_mut().take(), &answers) {
                (1, Some(rp), Some((total, _))) => rp.calls == *total && rp.answers.is_empty(),
                _ => true,
            };
            let key = if mode == 0 { tag } else { tag.to_ascii_lowercase() };
            let e = out.entry(key).or_insert([0usize; 2]);
            e[1] += 1;
            if ok {
                e[0] += 1;
            } else if bad.len() < 16 {
                bad.push(format!("[{key}] {}", line.chars().take(200).collect::<String>()));
            }
            }
        }
        (out, bad)
    }

    fn report(name: &str, text: &str) -> Option<std::collections::BTreeMap<char, [usize; 2]>> {
        let (counts, bad) = check(text);
        if counts.is_empty() {
            return None;
        }
        eprintln!("{name}: {counts:?}");
        for b in &bad {
            eprintln!("  differs: {b}");
        }
        Some(counts)
    }

    /// The rail helpers against the original's code run offline on the real railnet (work/racerai/crail, 30 cases of
    /// random car places / headings / speeds / offsets, the shared random numbers before and after): per case the
    /// nearest rail with heading (N), the walk on (A), the offset (O), the placement with the probe replayed (P), the
    /// goals (G), DriveToNearestRail (D), and every 20th lane's rail speed (S). Upper case: the port whole; lower case:
    /// the same with the original's lane slot 3 answers (F-15621, the curve search F-53741) replayed, which checks the
    /// rest - the quadtree walk and its order, the heading filter, the walk on, the offset, poses, probe, goals, state.
    #[test]
    fn cruise_rail_matches_the_original() {
        let Some(c) = report("small", include_str!("testdata/cruiserail_small.txt")) else { return };
        for (tag, [eq, n]) in c {
            assert!(n > 0, "{tag}");
            assert_eq!(eq, n, "{tag}");
        }
    }

    /// The same on the 20,000-case recording (work/racerai/crail/out_big.txt, if present) and every lane's speed.
    #[test]
    fn cruise_rail_matches_the_original_20000() {
        let Ok(text) = std::fs::read_to_string("D:/MCLA/work/racerai/crail/out_big.txt") else { return };
        let Some(c) = report("big", &text) else { return };
        for (tag, [eq, n]) in c {
            assert_eq!(eq, n, "{tag}");
        }
    }
}
