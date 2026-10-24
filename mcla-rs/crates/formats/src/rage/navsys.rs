//! Road network ("navsys", resource type 66, e.g. `resources/city/sc/0x9960fcd8.r66`).
//!
//! The network is a set of directed **lanes** (class 0x00479f0c, stride 0x70):
//! ```text
//! +04 flags (1 normal, 0x2001 special)   +08 owner (road section) ptr   +0c length
//! +10 vec3 bbox min   +20 vec3 bbox max
//! +30 the lane's curve (rage cvCurve, vtable 0x8202b390): +44 TRange, +48 TMax,
//!     +4c u16 CVCount - 1, +4e u8 Form, +4f u8 Degree, +50 -> u16 vertex indices
//!     (CVCount of them), +54 u16 knot vector index
//! ```
//! The vertex indices refer to the shared vec4 array at root+0x2c, the knot vectors to the
//! table at root+0x30 (12 bytes each: vtable, -> f32 knots, count; sub_8224B2B8 stores both
//! as globals 0x828DA6B4 / 0x828DA6B8 for the lane's vfuncs 2 / 1, sub_82244C80 /
//! sub_82244C60). A lane is a clamped uniform B-spline of its degree through its first and
//! last control point (complete edition: 1106 lanes of degree 1, 4291 of 2, 4634 of 3); its
//! last control point is another lane's first (every lane), and its stored length +0c is
//! the curve's (sub_82244E10). See [`LaneCurve`]. Lanes are owned by road
//! sections (0x0047a13c, stride 0xb0: +14 lane array, +18 count, +1c total length,
//! +24 centre/radius, +90 street name, +94 speed limit in m/s) or junction objects.
//!
//! For driving, every lane is split into its straight segments: a [`Link`] is one
//! segment, chained to the next segment of the lane or to the first segments of the
//! successor lanes. [`NavSys::parse`] makes the segments the lane's control polygon;
//! [`NavSys::curved`] a copy of the graph whose segments follow the curves.

use super::Rd;
use crate::rsc::Resource;
use crate::Result;
use glam::Vec3;
use hashbrown::HashMap;

#[derive(Debug, Clone)]
pub struct Link {
    pub a: Vec3,
    pub b: Vec3,
    pub len: f32,
    pub flags: u32,
    pub zone: Option<usize>,
    /// the owner object's (road section or junction) flags word +04 and word +10: the
    /// ambient traffic's rail speed reads them (sub_82245A28: 0x10 or 0x20 with +10 == 0
    /// 15 mph, 0x20 10 mph, 0x40 6 mph, else the road's limit)
    pub owner_flags: u32,
    pub owner_w10: u32,
    /// the owner's speed word +94 (m/s; a road section's limit, 0 = the default)
    pub owner_speed: f32,
    /// the flags of the owner's parent (owner +08 -> +04; sub_82245340 / sub_822452F8 look
    /// one level up for a road or a junction)
    pub parent_flags: u32,
    /// the road the owner belongs to when the owner is not a road itself (a connector lane
    /// of a road)
    pub parent_zone: Option<usize>,
    /// index of the lane this segment belongs to
    pub lane: usize,
    /// links that start where this one ends
    pub next: Vec<usize>,
    /// links that end where this one starts
    pub prev: Vec<usize>,
    /// lane changes: links of adjacent lanes (same direction) reachable from this link's end
    pub side: Vec<usize>,
    /// oncoming lanes next to this link's end (entered against traffic)
    pub opp: Vec<usize>,
    /// lanes next to this link's start that run opposite to it (used to get back on the
    /// correct side when this link is driven against traffic)
    pub back: Vec<usize>,
    /// junction hops: lane starts within reach of this link's end (lane ends only) that
    /// are not connected by a lane, e.g. turns the junction has no connector for
    pub hop: Vec<usize>,
    /// same, from this link's start when it is driven against traffic (lane starts only)
    pub hop_a: Vec<usize>,
    /// endpoint node ids (links sharing an endpoint share the node)
    pub node_a: usize,
    pub node_b: usize,
    /// the lane's unit tangent at `a` and `b` (on a [`NavSys::curved`] graph the curve's;
    /// else the segment's direction)
    pub ta: Vec3,
    pub tb: Vec3,
    /// the lane curve's phase at `a` and `b` (on the control polygon: the points' share of
    /// the lane's points, as the degree-1 curve)
    pub t: [f32; 2],
}

impl Link {
    pub fn dir(&self) -> Vec3 {
        (self.b - self.a).normalize_or(-Vec3::Z)
    }
    pub fn point_at(&self, t: f32) -> Vec3 {
        self.a.lerp(self.b, t.clamp(0.0, 1.0))
    }
    /// the lane's direction at `t` (0 .. 1 along the segment): the tangents at the ends
    /// blended (on a curved graph the curve's tangent to well under a degree)
    pub fn tangent_at(&self, t: f32) -> Vec3 {
        self.ta.lerp(self.tb, t.clamp(0.0, 1.0)).normalize_or(self.dir())
    }
    /// the lane's phase `s` metres along the link (and back: `s_at`)
    pub fn phase_at(&self, s: f32) -> f32 {
        self.t[0] + (self.t[1] - self.t[0]) * (s / self.len.max(1e-3)).clamp(0.0, 1.0)
    }
    pub fn s_at(&self, t: f32) -> f32 {
        let w = self.t[1] - self.t[0];
        if w <= 0.0 { 0.0 } else { ((t - self.t[0]) / w).clamp(0.0, 1.0) * self.len }
    }
    /// parameter of the closest point on the link to `p`
    pub fn project(&self, p: Vec3) -> f32 {
        let d = self.b - self.a;
        let l2 = d.length_squared().max(1e-6);
        ((p - self.a).dot(d) / l2).clamp(0.0, 1.0)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Zone {
    pub links: Vec<usize>,
    pub length: f32,
    pub center: Vec3,
    pub radius: f32,
    pub name: String,
    /// Authored road speed in m/s; zero selects the engine's default (40 mph).
    pub speed_limit: f32,
    /// the road's rectangle as the traffic spawner tests it (mcRnRoadAmbient +3c width, +40
    /// length, +50 centre, +60 direction: sub_826F4E90 / sub_8261B200)
    pub width: f32,
    pub rect_len: f32,
    pub mid: Vec3,
    pub dir: Vec3,
    /// across the road (+70), its height range (+44 .. +48) and the byte that leaves it
    /// without a box (+4c): the spawner's occlusion box (road vfunc 2, sub_8224D130)
    pub side: Vec3,
    pub y_range: [f32; 2],
    pub no_box: bool,
    /// the road's traffic density index from data (+98; -1: the default, 3)
    pub density: i32,
    /// the x / z the region's sphere test measures from (+34, +38: vfunc 3 sub_8224D4C0)
    pub rect_c: [f32; 2],
}

pub const DEFAULT_ROAD_SPEED: f32 = 40.0 * 0.44704;

impl Zone {
    pub fn speed_limit_mps(&self) -> f32 {
        // sub_822447B8 uses 40 mph when +148 is zero. Reject corrupt values too.
        if self.speed_limit.is_finite() && self.speed_limit > 0.0 {
            self.speed_limit
        } else {
            DEFAULT_ROAD_SPEED
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Street {
    pub name: String,
}

/// A lane's curve (lane +0x30, a rage cvCurve read by sub_82586388: Form, Degree, TRange,
/// TMax, CVCount): a clamped B-spline of `degree` over its control points with the knot
/// vector of the shared table.
#[derive(Debug, Clone, Default)]
pub struct LaneCurve {
    pub degree: u8,
    /// the control points (CVCount: the lane's u16 +0x4c plus one)
    pub pts: Vec<Vec3>,
    pub knots: Vec<f32>,
    pub t_range: f32,
    pub t_max: f32,
}

impl LaneCurve {
    fn linear(&self) -> bool {
        self.degree <= 1 || self.knots.len() != self.pts.len() + self.degree as usize + 1
    }

    /// The B-spline basis N(i, k) at u (sub_825870F0, Cox - de Boor; k = degree + 1; a
    /// zero knot span drops its term) and its derivative.
    fn basis(&self, i: usize, k: usize, u: f32) -> (f32, f32) {
        let kn = &self.knots;
        if k == 1 {
            return (if kn[i] <= u && u < kn[i + 1] { 1.0 } else { 0.0 }, 0.0);
        }
        let (a, b) = (kn[i + k - 1] - kn[i], kn[i + k] - kn[i + 1]);
        let (mut n, mut d) = (0.0, 0.0);
        if a != 0.0 {
            let n0 = self.basis(i, k - 1, u).0;
            n += (u - kn[i]) / a * n0;
            d += (k - 1) as f32 / a * n0;
        }
        if b != 0.0 {
            let n1 = self.basis(i + 1, k - 1, u).0;
            n += (kn[i + k] - u) / b * n1;
            d -= (k - 1) as f32 / b * n1;
        }
        (n, d)
    }

    /// The point at phase `t` (0 .. 1) and the derivative dP/dt (sub_82586A28; the point as
    /// sub_82587390): u = min(t x TRange, TMax) in the knot span holding it, the sum of the
    /// degree + 1 control points weighted by the basis. Degree 1 is the control polygon
    /// (t x (CVCount - 1), t = 1 read as 0.9999).
    pub fn eval(&self, t: f32) -> (Vec3, Vec3) {
        let t = t.clamp(0.0, 1.0);
        let n = self.pts.len();
        if n < 2 {
            return (self.pts.first().copied().unwrap_or(Vec3::ZERO), Vec3::ZERO);
        }
        if self.linear() {
            let t = if t >= 1.0 { 0.9999 } else { t };
            let f = (n - 1) as f32 * t;
            let i = (f as usize).min(n - 2);
            let (a, b) = (self.pts[i], self.pts[i + 1]);
            return (a + (b - a) * (f - i as f32), (b - a) * (n - 1) as f32);
        }
        let d = self.degree as usize;
        let u = (t * self.t_range).min(self.t_max);
        // the knot span (sub_82586510): the first j with K[j] <= u < K[j + 1]
        let kn = &self.knots;
        let mut j = 0;
        while j + 2 < kn.len() && !(kn[j] <= u && u < kn[j + 1]) {
            j += 1;
        }
        let j = j.clamp(d, n - 1);
        let (mut p, mut dp) = (Vec3::ZERO, Vec3::ZERO);
        for i in j - d..=j {
            let (w, dw) = self.basis(i, d + 1, u);
            p += self.pts[i] * w;
            dp += self.pts[i] * dw;
        }
        (p, dp * self.t_range)
    }

    /// |dP/dt| at `t` (lane vfunc 11, sub_82586CD0)
    pub fn speed(&self, t: f32) -> f32 {
        self.eval(t).1.length()
    }

    /// The phase after `d` metres at the curve speed `s` (lane vfunc 6 sub_82245CD0 ->
    /// sub_82586598): t + d / s; s <= 1e-6: 0.01 the way of `d`.
    pub fn advance(t: f32, d: f32, s: f32) -> f32 {
        if s > 1e-6 {
            t + d / s
        } else if d > 0.0 {
            t + 0.01
        } else {
            t - 0.01
        }
    }

    /// The phase of the curve's point nearest to `p` between phases `t0` and `t1` (as lane
    /// slot 3 sub_82245388 -> sub_825875F0 finds it: trial points, then 10 refinements;
    /// here 16 trial points and 10 halvings of the bracket round the best).
    pub fn nearest(&self, p: Vec3, t0: f32, t1: f32) -> f32 {
        let (t0, t1) = (t0.clamp(0.0, 1.0), t1.clamp(0.0, 1.0));
        if t1 <= t0 {
            return t0;
        }
        let d = |t: f32| self.eval(t).0.distance_squared(p);
        let n = 16;
        let h = (t1 - t0) / n as f32;
        let mut best = (d(t0), t0);
        for k in 1..=n {
            let t = t0 + h * k as f32;
            let e = d(t);
            if e < best.0 {
                best = (e, t);
            }
        }
        let (mut a, mut b) = ((best.1 - h).max(t0), (best.1 + h).min(t1));
        for _ in 0..10 {
            let (m1, m2) = (a + (b - a) / 3.0, b - (b - a) / 3.0);
            if d(m1) < d(m2) {
                b = m2;
            } else {
                a = m1;
            }
        }
        (a + b) * 0.5
    }

    /// The curve's length as the original measures it (sub_82244E10): 100 chords at
    /// t = 0.01 k for degree 2 and up, else the control polygon.
    pub fn length(&self) -> f32 {
        if self.linear() {
            return self.pts.windows(2).map(|w| w[0].distance(w[1])).sum();
        }
        let mut q = self.eval(0.0).0;
        let mut len = 0.0;
        for k in 1..=100 {
            let r = self.eval(0.01 * k as f32).0;
            len += q.distance(r);
            q = r;
        }
        len
    }

    /// Points along the curve and its unit tangents there: the control polygon for degree 1
    /// (no tangents: the segments' own directions), else the curve's start, every trial point
    /// (64 a knot span) after which the tangent would have turned more than `max_turn`
    /// radians since the last point kept, and the last control point (the curve ends there).
    pub fn samples(&self, max_turn: f32) -> (Vec<Vec3>, Vec<Vec3>, Vec<f32>) {
        if self.linear() || self.pts.len() < 2 {
            return (self.pts.clone(), Vec::new(), Vec::new());
        }
        let spans = (self.pts.len() - self.degree as usize).max(1);
        let m = 64 * spans;
        let at = |t: f32| {
            let (p, d) = self.eval(t);
            (p, d.normalize_or(Vec3::ZERO))
        };
        let cos_max = max_turn.cos();
        let (p0, t0) = at(0.0);
        let (mut pts, mut tans, mut ts) = (vec![p0], vec![t0], vec![0.0]);
        let mut prev = (p0, t0, 0.0);
        for k in 1..m {
            let tk = k as f32 / m as f32;
            let cur = at(tk);
            if cur.1.dot(*tans.last().unwrap()) < cos_max && prev.0 != *pts.last().unwrap() {
                pts.push(prev.0);
                tans.push(prev.1);
                ts.push(prev.2);
            }
            prev = (cur.0, cur.1, tk);
        }
        let end = *self.pts.last().unwrap();
        let te = at(1.0).1;
        if pts.last().unwrap().distance(end) > 1e-4 {
            pts.push(end);
            tans.push(if te != Vec3::ZERO { te } else { prev.1 });
            ts.push(1.0);
        } else {
            *ts.last_mut().unwrap() = 1.0;
        }
        (pts, tans, ts)
    }
}

#[derive(Debug, Default)]
pub struct NavSys {
    pub links: Vec<Link>,
    pub zones: Vec<Zone>,
    pub streets: Vec<Street>,
    /// number of lanes in the file
    pub lanes: usize,
    /// every lane's curve, by lane index
    pub curves: Vec<LaneCurve>,
    /// the lanes as read and the roads' lane lists (for [`NavSys::curved`])
    raw: Vec<RawLane>,
    zone_lanes: Vec<Vec<usize>>,
    grid: HashMap<(i32, i32), Vec<usize>>,
    /// per endpoint node: links starting / ending there
    starts_at: Vec<Vec<usize>>,
    ends_at: Vec<Vec<usize>>,
    /// per lane, the junction it belongs to (sub_822452F8: the lane's owner +08 when the owner's
    /// flags +04 have bit 4, else the owner's parent +08 when its flags have bit 4), as an index
    /// into the junction array (root +58, 0xc0 bytes each, count root +c8)
    pub lane_junction: Vec<Option<usize>>,
    /// per junction of that array: its centre (+80) and its rails as sub_8273C1B8 walks them -
    /// its own lanes (+14 array, +18 u16 count), then its first child (+0c) and, below the
    /// junction, each node's next sibling (+10), depth first
    pub junction_centers: Vec<Vec3>,
    pub junction_lanes: Vec<Vec<usize>>,
    /// the lane quadtree (root +130; node class 0x47b4dc), node 0 the root
    pub quad: Vec<QuadNode>,
    /// the longest link (`path_to_point`'s look-up of the links near its goal)
    max_link_len: f32,
    /// the graph's strongly connected components, made on first use (`path_to_point`)
    sccs: std::sync::OnceLock<Sccs>,
}

/// The lane graph's strongly connected components over the edges `path_to_point` walks (`next` and `side`) and
/// the graph between them: with them `path_to_point` knows at once when no link it could stop on can be reached
/// from its start, the case in which its search walks every link it can reach before it gives up
/// (docs/PERFORMANCE.md 2026-10-23 (6): police cars chasing a player off the lanes, every tick).
#[derive(Debug, Default)]
struct Sccs {
    comp: Vec<u32>,
    succ: Vec<Vec<u32>>,
}

/// A node of the lane quadtree (root +130): +04 -> its lanes, +08 u16 their number, +1c .. +28
/// the four children, +2c / +30 the x / z it is split at, +34 / +38 the box's least x / z, +3c /
/// +40 its greatest x / z.
#[derive(Debug, Clone, Default)]
pub struct QuadNode {
    pub lanes: Vec<usize>,
    pub kids: [Option<usize>; 4],
    pub split: [f32; 2],
    pub min: [f32; 2],
    pub max: [f32; 2],
}

const LINK_VT: u32 = 0x00479f0c;
const ZONE_VT: u32 = 0x0047a13c;
const STREET_VT: u32 = 0x0047a0c4;
const GRID: f32 = 50.0;

fn cell(p: Vec3) -> (i32, i32) {
    ((p.x / GRID).floor() as i32, (p.z / GRID).floor() as i32)
}

fn xz(a: Vec3, b: Vec3) -> f32 {
    ((a.x - b.x).powi(2) + (a.z - b.z).powi(2)).sqrt()
}

#[derive(Debug, Clone)]
struct RawLane {
    pts: Vec<Vec3>,
    /// unit tangents and the curve's phases at `pts` (empty: the segments' directions, the
    /// points' share)
    tans: Vec<Vec3>,
    ts: Vec<f32>,
    /// the first and last control points' vertex indices
    vi: (u16, u16),
    curve: LaneCurve,
    min: Vec3,
    max: Vec3,
    len: f32,
    flags: u32,
    zone: Option<usize>,
    owner: (u32, u32, f32, u32),
    parent_zone: Option<usize>,
}

impl NavSys {
    pub fn parse(res: &Resource) -> Result<NavSys> {
        let rd = Rd::new(res);
        let vs = res.virtual_size;
        let mut nav = NavSys::default();
        let mut link_offs = Vec::new();
        let mut zone_offs = Vec::new();
        let mut street_offs = Vec::new();
        let mut o = 0usize;
        while o + 0x30 <= vs {
            match rd.vtable(o) {
                LINK_VT => {
                    link_offs.push(o);
                    o += 0x70;
                    continue;
                }
                ZONE_VT => {
                    zone_offs.push(o);
                    o += 0xb0;
                    continue;
                }
                STREET_VT => {
                    street_offs.push(o);
                    o += 0xc0;
                    continue;
                }
                _ => {}
            }
            o += 4;
        }
        let verts = rd.ptr(0x2c);
        let vert = |i: usize| -> Option<Vec3> {
            let v = verts? + i * 16;
            if !rd.has(v, 16) {
                return None;
            }
            Some(Vec3::new(rd.f32(v), rd.f32(v + 4), rd.f32(v + 8)))
        };
        let zone_index = |off: usize| zone_offs.binary_search(&off).ok();
        // the knot vectors (root +0x30, 12 bytes each: vtable, -> knots, count)
        let knots = |i: usize| -> Vec<f32> {
            let Some(o) = rd.ptr(0x30).map(|t| t + i * 12).filter(|&o| rd.has(o, 12)) else { return Vec::new() };
            let n = rd.u32(o + 8) as usize;
            match rd.ptr(o + 4) {
                Some(k) if n <= 256 && rd.has(k, n * 4) => (0..n).map(|j| rd.f32(k + j * 4)).collect(),
                _ => Vec::new(),
            }
        };
        let mut raw: Vec<RawLane> = Vec::with_capacity(link_offs.len());
        for &lo in &link_offs {
            let min = Vec3::new(rd.f32(lo + 0x10), rd.f32(lo + 0x14), rd.f32(lo + 0x18));
            let max = Vec3::new(rd.f32(lo + 0x20), rd.f32(lo + 0x24), rd.f32(lo + 0x28));
            // CVCount - 1 at +4c: the lane lists CVCount vertex indices, its last point too
            let cnt = rd.u16(lo + 0x4c) as usize + 1;
            let mut pts = Vec::with_capacity(cnt);
            let mut vi = (u16::MAX, u16::MAX);
            if let Some(arr) = rd.ptr(lo + 0x50) {
                for k in 0..cnt.min(65) {
                    if !rd.has(arr + k * 2, 2) {
                        break;
                    }
                    let ix = rd.u16(arr + k * 2);
                    if let Some(p) = vert(ix as usize) {
                        pts.push(p);
                        if k == 0 {
                            vi.0 = ix;
                        }
                        vi.1 = ix;
                    }
                }
            }
            if pts.is_empty() {
                pts.push(min);
            }
            let curve = LaneCurve { degree: rd.u8(lo + 0x4f), pts: pts.clone(), knots: knots(rd.u16(lo + 0x54) as usize), t_range: rd.f32(lo + 0x44), t_max: rd.f32(lo + 0x48) };
            raw.push(RawLane { pts, tans: Vec::new(), ts: Vec::new(), vi, curve, min, max, len: rd.f32(lo + 0xc), flags: rd.u32(lo + 4), zone: rd.ptr(lo + 8).and_then(zone_index), owner: rd.ptr(lo + 8).filter(|&o| rd.has(o, 0x98)).map_or((0, 0, 0.0, 0), |o| (rd.u32(o + 4), rd.u32(o + 0x10), rd.f32(o + 0x94), rd.ptr(o + 8).filter(|&p| rd.has(p, 8)).map_or(0, |p| rd.u32(p + 4)))), parent_zone: rd.ptr(lo + 8).filter(|&o| rd.has(o, 0x10)).and_then(|o| rd.ptr(o + 8)).and_then(zone_index) });
        }
        let link_index = |off: usize| link_offs.binary_search(&off).ok();
        let mut zone_lanes: Vec<Vec<usize>> = Vec::new();
        for &zo in &zone_offs {
            let mut z = Zone { length: rd.f32(zo + 0x1c), center: Vec3::new(rd.f32(zo + 0x24), rd.f32(zo + 0x28), rd.f32(zo + 0x2c)), radius: rd.f32(zo + 0x30), ..Default::default() };
            z.speed_limit = rd.f32(zo + 0x94);
            z.width = rd.f32(zo + 0x3c);
            z.rect_len = rd.f32(zo + 0x40);
            z.mid = Vec3::new(rd.f32(zo + 0x50), rd.f32(zo + 0x54), rd.f32(zo + 0x58));
            z.dir = Vec3::new(rd.f32(zo + 0x60), rd.f32(zo + 0x64), rd.f32(zo + 0x68));
            z.side = Vec3::new(rd.f32(zo + 0x70), rd.f32(zo + 0x74), rd.f32(zo + 0x78));
            z.y_range = [rd.f32(zo + 0x44), rd.f32(zo + 0x48)];
            z.no_box = rd.u8(zo + 0x4c) != 0;
            z.density = rd.u32(zo + 0x98) as i32;
            z.rect_c = [rd.f32(zo + 0x34), rd.f32(zo + 0x38)];
            if let Some(np) = rd.ptr(zo + 0x90) {
                let s = rd.cstr(np);
                if s.len() < 64 && s.bytes().all(|b| (0x20..0x7f).contains(&b)) {
                    z.name = s;
                }
            }
            let count = rd.u16(zo + 0x18) as usize;
            let mut lanes = Vec::new();
            if let Some(arr) = rd.ptr(zo + 0x14) {
                for i in 0..count.min(4096) {
                    if let Some(li) = rd.ptr(arr + i * 4).and_then(link_index) {
                        lanes.push(li);
                    }
                }
            }
            zone_lanes.push(lanes);
            nav.zones.push(z);
        }
        for &so in &street_offs {
            let mut name = String::new();
            for f in (4..0xc0).step_by(4) {
                if let Some(p) = rd.ptr(so + f) {
                    let s = rd.cstr(p);
                    if s.len() >= 3 && s.len() < 64 && s.bytes().all(|b| (0x20..0x7f).contains(&b)) && s.chars().any(|c| c.is_ascii_alphabetic()) {
                        name = s;
                        break;
                    }
                }
            }
            nav.streets.push(Street { name });
        }
        nav.curves = raw.iter().map(|l| l.curve.clone()).collect();
        nav.raw = raw.clone();
        nav.zone_lanes = zone_lanes.clone();
        // the lanes' junctions (sub_822452F8) and the junctions' rails (sub_8273C1B8)
        let jarr = rd.ptr(0x58);
        let jn = (rd.u32(0xc8) as usize).min(4096);
        let jindex = |o: usize| -> Option<usize> {
            let a = jarr?;
            (o >= a && (o - a) % 0xc0 == 0 && (o - a) / 0xc0 < jn).then(|| (o - a) / 0xc0)
        };
        nav.lane_junction = link_offs
            .iter()
            .map(|&lo| {
                let ow = rd.ptr(lo + 8)?;
                let j = if rd.u32(ow + 4) & 4 != 0 { ow } else { rd.ptr(ow + 8).filter(|&p| rd.u32(p + 4) & 4 != 0)? };
                jindex(j)
            })
            .collect();
        fn walk(rd: &Rd, node: usize, mut below: bool, li: &dyn Fn(usize) -> Option<usize>, out: &mut Vec<usize>, depth: usize) {
            let mut n = Some(node);
            while let Some(o) = n {
                if depth > 64 {
                    return;
                }
                if let Some(a) = rd.ptr(o + 0x14) {
                    for k in 0..rd.u16(o + 0x18) as usize {
                        if let Some(l) = rd.ptr(a + k * 4).and_then(li) {
                            out.push(l);
                        }
                    }
                }
                if below {
                    if let Some(s) = rd.ptr(o + 0x10) {
                        walk(rd, s, true, li, out, depth + 1);
                    }
                }
                n = rd.ptr(o + 0x0c);
                below = true;
            }
        }
        for i in 0..jn {
            let Some(o) = jarr.map(|a| a + i * 0xc0).filter(|&o| rd.has(o, 0xc0)) else { break };
            nav.junction_centers.push(Vec3::new(rd.f32(o + 0x80), rd.f32(o + 0x84), rd.f32(o + 0x88)));
            let mut out = Vec::new();
            walk(&rd, o, false, &link_index, &mut out, 0);
            nav.junction_lanes.push(out);
        }
        // the lane quadtree (root +130)
        fn quad(rd: &Rd, o: usize, li: &dyn Fn(usize) -> Option<usize>, out: &mut Vec<QuadNode>, depth: usize) -> usize {
            let me = out.len();
            let mut q = QuadNode { split: [rd.f32(o + 0x2c), rd.f32(o + 0x30)], min: [rd.f32(o + 0x34), rd.f32(o + 0x38)], max: [rd.f32(o + 0x3c), rd.f32(o + 0x40)], ..Default::default() };
            if let Some(a) = rd.ptr(o + 4) {
                for k in 0..rd.u16(o + 8) as usize {
                    if let Some(l) = rd.ptr(a + k * 4).and_then(li) {
                        q.lanes.push(l);
                    }
                }
            }
            out.push(q);
            if depth < 32 {
                for k in 0..4 {
                    if let Some(c) = rd.ptr(o + 0x1c + k * 4).filter(|&c| rd.has(c, 0x44)) {
                        let ci = quad(rd, c, li, out, depth + 1);
                        out[me].kids[k] = Some(ci);
                    }
                }
            }
            me
        }
        if let Some(r) = rd.ptr(0x130).filter(|&r| rd.has(r, 0x44)) {
            let mut q = Vec::new();
            quad(&rd, r, &link_index, &mut q, 0);
            nav.quad = q;
        }
        nav.build_graph(raw, zone_lanes);
        Ok(nav)
    }

    /// A copy of the graph whose links follow the lanes' curves (a new point wherever the
    /// tangent turned more than `max_turn` radians, with the curve's tangents at the points),
    /// as the original's rail follower rides them (sub_827986C0: the lane's vfunc 9,
    /// sub_82586A28, at the follower's phase). The lanes, their order and successors are
    /// those of this graph; the link indices are its own.
    pub fn curved(&self, max_turn: f32) -> NavSys {
        let raw: Vec<RawLane> = self
            .raw
            .iter()
            .map(|l| {
                let (pts, tans, ts) = l.curve.samples(max_turn);
                if pts.len() < 2 {
                    return l.clone();
                }
                RawLane { pts, tans, ts, ..l.clone() }
            })
            .collect();
        let mut zones = self.zones.clone();
        for z in &mut zones {
            z.links.clear();
        }
        let mut nav = NavSys { zones, streets: self.streets.clone(), curves: self.curves.clone(), raw: self.raw.clone(), zone_lanes: self.zone_lanes.clone(), lane_junction: self.lane_junction.clone(), junction_centers: self.junction_centers.clone(), junction_lanes: self.junction_lanes.clone(), quad: self.quad.clone(), ..Default::default() };
        nav.build_graph(raw, self.zone_lanes.clone());
        nav
    }

    fn build_graph(&mut self, mut raw: Vec<RawLane>, zone_lanes: Vec<Vec<usize>>) {
        let n = raw.len();
        self.lanes = n;
        // lane starts, for end matching
        let scell = |p: Vec3| ((p.x / 8.0).floor() as i32, (p.z / 8.0).floor() as i32);
        let mut sgrid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (i, l) in raw.iter().enumerate() {
            sgrid.entry(scell(l.pts[0])).or_default().push(i);
        }
        let starts_near = |raw: &Vec<RawLane>, p: Vec3, r: f32, me: usize| -> Vec<(f32, usize)> {
            let (cx, cz) = scell(p);
            let k = (r / 8.0).ceil() as i32;
            let mut v = Vec::new();
            for dx in -k..=k {
                for dz in -k..=k {
                    if let Some(list) = sgrid.get(&(cx + dx, cz + dz)) {
                        for &j in list {
                            if j == me {
                                continue;
                            }
                            let q = raw[j].pts[0];
                            let d = xz(q, p);
                            if d < r && (q.y - p.y).abs() < 4.0 {
                                v.push((d, j));
                            }
                        }
                    }
                }
            }
            v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            v
        };
        // lanes by their first control point's vertex
        let mut by_first: HashMap<u16, Vec<usize>> = HashMap::new();
        for (i, l) in raw.iter().enumerate() {
            if l.vi.0 != u16::MAX {
                by_first.entry(l.vi.0).or_default().push(i);
            }
        }
        // 1. end point of every lane + its successors
        let mut ends: Vec<Vec3> = Vec::with_capacity(n);
        let mut succ: Vec<Vec<usize>> = Vec::with_capacity(n);
        for i in 0..n {
            let l = &raw[i];
            let s = l.pts[0];
            let last = *l.pts.last().unwrap();
            // the last control point; the successors: the lanes starting at its vertex (the
            // original's graph node, sub_8273BD80). (Without the index: the corner of the
            // bounding box opposite the start, and lane starts near it.)
            let mut end = last;
            let mut found: Vec<usize> = by_first.get(&l.vi.1).map(|v| v.iter().copied().filter(|&j| j != i).collect()).unwrap_or_default();
            if found.is_empty() && l.vi.1 == u16::MAX {
                let ex = if (s.x - l.min.x).abs() <= (s.x - l.max.x).abs() { l.max.x } else { l.min.x };
                let ez = if (s.z - l.min.z).abs() <= (s.z - l.max.z).abs() { l.max.z } else { l.min.z };
                end = Vec3::new(ex, last.y, ez);
            }
            if found.is_empty() {
                found = starts_near(&raw, end, 1.0, i).into_iter().map(|x| x.1).collect();
            }
            if found.is_empty() {
                // curved lane (the box corner is not on it): look for a lane start ahead of
                // the last listed point at the distance the remaining length predicts
                let listed: f32 = l.pts.windows(2).map(|w| w[0].distance(w[1])).sum();
                let remaining = (l.len - listed).max(0.0);
                let fwd = if l.pts.len() >= 2 { (last - l.pts[l.pts.len() - 2]).normalize_or(Vec3::Z) } else { (end - s).normalize_or(Vec3::Z) };
                let mut best: Option<(f32, usize)> = None;
                for (_, j) in starts_near(&raw, last, remaining + 12.0, i) {
                    let q = raw[j].pts[0];
                    if q.x < l.min.x - 1.5 || q.x > l.max.x + 1.5 || q.z < l.min.z - 1.5 || q.z > l.max.z + 1.5 {
                        continue;
                    }
                    let v = q - last;
                    let d = v.length();
                    if d < 0.5 || v.dot(fwd) / d.max(0.01) < 0.3 {
                        continue;
                    }
                    let err = (d - remaining).abs();
                    if err < 6.0 + remaining * 0.12 && best.map_or(true, |b| err < b.0) {
                        best = Some((err, j));
                    }
                }
                if let Some((_, j)) = best {
                    end = raw[j].pts[0];
                    found = starts_near(&raw, end, 1.0, i).into_iter().map(|x| x.1).collect();
                    if !found.contains(&j) {
                        found.push(j);
                    }
                }
            }
            if found.is_empty() {
                // no exact continuation: take compatible lane starts close to the end
                let d0 = (end - last).normalize_or(Vec3::Z);
                found = starts_near(&raw, end, 7.0, i)
                    .into_iter()
                    .filter(|&(_, j)| {
                        let l2 = &raw[j];
                        let nd = if l2.pts.len() >= 2 { (l2.pts[1] - l2.pts[0]).normalize_or(d0) } else { d0 };
                        nd.dot(d0) > 0.2
                    })
                    .take(3)
                    .map(|x| x.1)
                    .collect();
            }
            ends.push(end);
            succ.push(found);
        }
        // 2. split lanes into straight links
        let mut first = vec![0usize; n];
        let mut lastl = vec![0usize; n];
        let mut links: Vec<Link> = Vec::with_capacity(n * 4);
        for i in 0..n {
            let l = &mut raw[i];
            if l.pts.last().map_or(true, |p| p.distance(ends[i]) > 0.05) {
                l.pts.push(ends[i]);
            }
            if l.pts.len() < 2 {
                l.pts.push(ends[i] + Vec3::new(0.01, 0.0, 0.0));
            }
            if l.tans.len() != l.pts.len() || l.ts.len() != l.pts.len() {
                l.tans.clear();
                l.ts.clear();
            }
            let np = (l.pts.len() - 1).max(1) as f32;
            first[i] = links.len();
            for (j, w) in l.pts.windows(2).enumerate() {
                let k = links.len();
                let dir = (w[1] - w[0]).normalize_or(-Vec3::Z);
                let (ta, tb) = if l.tans.is_empty() { (dir, dir) } else { (l.tans[j], l.tans[j + 1]) };
                let t = if l.ts.is_empty() { [j as f32 / np, (j + 1) as f32 / np] } else { [l.ts[j], l.ts[j + 1]] };
                links.push(Link { a: w[0], b: w[1], len: w[0].distance(w[1]).max(0.01), flags: l.flags, zone: l.zone, owner_flags: l.owner.0, owner_w10: l.owner.1, owner_speed: l.owner.2, parent_flags: l.owner.3, parent_zone: l.parent_zone, lane: i, next: vec![k + 1], prev: Vec::new(), side: Vec::new(), opp: Vec::new(), back: Vec::new(), hop: Vec::new(), hop_a: Vec::new(), node_a: 0, node_b: 0, ta, tb, t });
            }
            lastl[i] = links.len() - 1;
        }
        for i in 0..n {
            let mut nx: Vec<usize> = succ[i].iter().map(|&j| first[j]).collect();
            nx.sort_unstable();
            nx.dedup();
            links[lastl[i]].next = nx;
        }
        for k in 0..links.len() {
            let nexts = links[k].next.clone();
            for j in nexts {
                links[j].prev.push(k);
            }
        }
        for (zi, lanes) in zone_lanes.into_iter().enumerate() {
            for li in lanes {
                if li < n {
                    self.zones[zi].links.extend(first[li]..=lastl[li]);
                }
            }
        }
        self.grid.clear();
        for (i, l) in links.iter().enumerate() {
            let c = cell(l.a);
            self.grid.entry(c).or_default().push(i);
            let c2 = cell(l.b);
            if c2 != c {
                self.grid.entry(c2).or_default().push(i);
            }
        }
        self.max_link_len = links.iter().map(|l| l.len).fold(0.0, f32::max);
        self.sccs = Default::default();
        self.links = links;
        // lane changes: from the end of a link onto a parallel link of a neighbouring lane
        let nl = self.links.len();
        let mut side: Vec<Vec<usize>> = vec![Vec::new(); nl];
        let mut opp: Vec<Vec<usize>> = vec![Vec::new(); nl];
        let mut back: Vec<Vec<usize>> = vec![Vec::new(); nl];
        for (i, l) in self.links.iter().enumerate() {
            let d = l.dir();
            for j in self.links_near(l.b, 10.0) {
                let m = &self.links[j];
                if j == i || m.lane == l.lane {
                    continue;
                }
                let dot = m.dir().dot(d);
                if dot.abs() < 0.92 {
                    continue;
                }
                // neighbours of the end point
                let t = m.project(l.b);
                let q = m.point_at(t);
                let lat = xz(q, l.b);
                if (2.0..6.5).contains(&lat) && (q.y - l.b.y).abs() < 2.0 {
                    if dot > 0.0 && t < 0.9 {
                        side[i].push(j);
                    } else if dot < 0.0 && t > 0.1 {
                        opp[i].push(j);
                    }
                }
                // neighbours of the start point that run the other way
                if dot < 0.0 {
                    let t = m.project(l.a);
                    let q = m.point_at(t);
                    let lat = xz(q, l.a);
                    if (2.0..6.5).contains(&lat) && (q.y - l.a.y).abs() < 2.0 && t < 0.9 {
                        back[i].push(j);
                    }
                }
            }
        }
        for i in 0..nl {
            self.links[i].side = std::mem::take(&mut side[i]);
            self.links[i].opp = std::mem::take(&mut opp[i]);
            self.links[i].back = std::mem::take(&mut back[i]);
        }
        // junction hops between lane ends and lane starts (open junction area)
        {
            const HOP: f32 = 32.0;
            let hcell = |p: Vec3| ((p.x / 16.0).floor() as i32, (p.z / 16.0).floor() as i32);
            let is_start: Vec<bool> = (0..nl).map(|i| i == 0 || self.links[i - 1].lane != self.links[i].lane).collect();
            let is_end: Vec<bool> = (0..nl).map(|i| i + 1 == nl || self.links[i + 1].lane != self.links[i].lane).collect();
            let mut sg: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
            for i in 0..nl {
                if is_start[i] {
                    sg.entry(hcell(self.links[i].a)).or_default().push(i);
                }
            }
            let near = |p: Vec3, travel: Vec3, me: usize, links: &Vec<Link>| -> Vec<usize> {
                let (cx, cz) = hcell(p);
                let mut v: Vec<(f32, usize)> = Vec::new();
                for dx in -2..=2 {
                    for dz in -2..=2 {
                        if let Some(list) = sg.get(&(cx + dx, cz + dz)) {
                            for &k in list {
                                let q = links[k].a;
                                let d = xz(q, p);
                                if k == me || links[k].lane == links[me].lane || d < 1.2 || d > HOP || (q.y - p.y).abs() > 2.5 {
                                    continue;
                                }
                                // not behind us, and not a lane that heads straight back
                                if (q - p).dot(travel) < -0.2 * d || links[k].dir().dot(travel) < -0.8 {
                                    continue;
                                }
                                v.push((d, k));
                            }
                        }
                    }
                }
                v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
                v.into_iter().take(64).map(|x| x.1).collect()
            };
            let mut hop: Vec<Vec<usize>> = vec![Vec::new(); nl];
            let mut hop_a: Vec<Vec<usize>> = vec![Vec::new(); nl];
            for i in 0..nl {
                if is_end[i] {
                    let l = &self.links[i];
                    hop[i] = near(l.b, l.dir(), i, &self.links).into_iter().filter(|k| !l.next.contains(k)).collect();
                }
                if is_start[i] {
                    let l = &self.links[i];
                    hop_a[i] = near(l.a, -l.dir(), i, &self.links);
                }
            }
            for i in 0..nl {
                self.links[i].hop = std::mem::take(&mut hop[i]);
                self.links[i].hop_a = std::mem::take(&mut hop_a[i]);
            }
        }
        // endpoint nodes (endpoints within 1.2 m)
        let pts: Vec<Vec3> = (0..nl * 2).map(|e| if e % 2 == 0 { self.links[e / 2].a } else { self.links[e / 2].b }).collect();
        let mut parent: Vec<usize> = (0..nl * 2).collect();
        fn find(p: &mut Vec<usize>, mut x: usize) -> usize {
            while p[x] != x {
                p[x] = p[p[x]];
                x = p[x];
            }
            x
        }
        let ncell = |p: Vec3| ((p.x / 4.0).floor() as i32, (p.z / 4.0).floor() as i32);
        let mut ng: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for e in 0..nl * 2 {
            ng.entry(ncell(pts[e])).or_default().push(e);
        }
        for e in 0..nl * 2 {
            let p = pts[e];
            let (cx, cz) = ncell(p);
            for dx in -1..=1 {
                for dz in -1..=1 {
                    if let Some(list) = ng.get(&(cx + dx, cz + dz)) {
                        for &f in list {
                            if f > e && xz(pts[f], p) < 1.2 && (pts[f].y - p.y).abs() < 3.0 {
                                let (ra, rb) = (find(&mut parent, e), find(&mut parent, f));
                                if ra != rb {
                                    parent[ra] = rb;
                                }
                            }
                        }
                    }
                }
            }
        }
        let mut ids: HashMap<usize, usize> = HashMap::new();
        self.starts_at.clear();
        self.ends_at.clear();
        for e in 0..nl * 2 {
            let r = find(&mut parent, e);
            let next_id = ids.len();
            let id = *ids.entry(r).or_insert(next_id);
            if id == self.starts_at.len() {
                self.starts_at.push(Vec::new());
                self.ends_at.push(Vec::new());
            }
            if e % 2 == 0 {
                self.links[e / 2].node_a = id;
                self.starts_at[id].push(e / 2);
            } else {
                self.links[e / 2].node_b = id;
                self.ends_at[id].push(e / 2);
            }
        }
    }

    /// A lane's box as the file has it (lane +10 least, +20 greatest corner).
    pub fn lane_box(&self, lane: usize) -> Option<(Vec3, Vec3)> {
        self.raw.get(lane).map(|l| (l.min, l.max))
    }

    /// (lanes, links, links with a successor)
    /// The lanes near `p` as the lane quadtree answers (sub_82246DB8, from sub_82249210): a node
    /// is entered when the x / z distance from `p` to its box, squared, is at most w^2
    /// (sub_82246B78); its children first, in the order of the table at 0x827DC710 by p's
    /// quadrant (q = 1 when p.z - split z > -1e-6, + 2 when p.x - split x > -1e-6; q 0: 0 1 2 3,
    /// 1: 1 3 0 2, 2: 2 0 3 1, 3: 3 2 1 0), then its own lanes whose flags (+4) meet `mask` and
    /// whose box (+10 / +20) is within w in 3D (lane slot 5 sub_82244DC0: w^2 > the squared
    /// distance), until `max` are found.
    pub fn lanes_near(&self, p: Vec3, w: f32, mask: u32, max: usize) -> Vec<usize> {
        const ORDER: [[usize; 4]; 4] = [[0, 1, 2, 3], [1, 3, 0, 2], [2, 0, 3, 1], [3, 2, 1, 0]];
        fn go(nav: &NavSys, n: usize, p: Vec3, w: f32, mask: u32, max: usize, out: &mut Vec<usize>) {
            let q = &nav.quad[n];
            let mut d = 0.0f32;
            if p.x < q.min[0] {
                d = (p.x - q.min[0]) * (p.x - q.min[0]);
            } else if p.x > q.max[0] {
                d = (p.x - q.max[0]) * (p.x - q.max[0]);
            }
            if p.z < q.min[1] {
                d = (p.z - q.min[1]).mul_add(p.z - q.min[1], d);
            } else if p.z > q.max[1] {
                d = (p.z - q.max[1]).mul_add(p.z - q.max[1], d);
            }
            if d > w * w {
                return;
            }
            let k = -9.999_999_7e-7f32;
            let quad = (if p.z - q.split[1] > k { 1 } else { 0 }) | (if p.x - q.split[0] > k { 2 } else { 0 });
            for &slot in &ORDER[quad] {
                if out.len() >= max {
                    break;
                }
                if let Some(c) = q.kids[slot] {
                    go(nav, c, p, w, mask, max, out);
                }
            }
            for &l in &q.lanes {
                if out.len() >= max {
                    break;
                }
                let r = &nav.raw[l];
                if r.flags & mask == 0 {
                    continue;
                }
                let e = (r.min - p).max(p - r.max).max(Vec3::ZERO);
                if w * w > e.length_squared() {
                    out.push(l);
                }
            }
        }
        let mut out = Vec::new();
        if !self.quad.is_empty() {
            go(self, 0, p, w, mask, max, &mut out);
        }
        out
    }

    pub fn stats(&self) -> (usize, usize, usize) {
        (self.lanes, self.links.len(), self.links.iter().filter(|l| !l.next.is_empty()).count())
    }

    /// Number of links reachable (forward) from `from`.
    pub fn reachable(&self, from: usize) -> usize {
        let mut seen = vec![false; self.links.len()];
        let mut st = vec![from];
        seen[from] = true;
        let mut c = 0;
        while let Some(u) = st.pop() {
            c += 1;
            for &v in &self.links[u].next {
                if !seen[v] {
                    seen[v] = true;
                    st.push(v);
                }
            }
        }
        c
    }

    /// Links near a point (grid cells around it).
    pub fn links_near(&self, p: Vec3, radius: f32) -> impl Iterator<Item = usize> + '_ {
        let (cx, cz) = cell(p);
        let r = (radius / GRID).ceil() as i32 + 1;
        let mut v = Vec::new();
        for dx in -r..=r {
            for dz in -r..=r {
                if let Some(list) = self.grid.get(&(cx + dx, cz + dz)) {
                    v.extend_from_slice(list);
                }
            }
        }
        v.sort_unstable();
        v.dedup();
        v.into_iter()
    }

    /// Effective speed limit of a road-owned link, in metres per second.
    pub fn speed_limit_mps(&self, link: usize) -> Option<f32> {
        self.links.get(link)?.zone.and_then(|z| self.zones.get(z)).map(Zone::speed_limit_mps)
    }

    /// Speed of the road under the car. Junctions/off-road return None so cruise
    /// can retain its previous limit, as sub_822A25E0 does without a road object.
    /// This proximity lookup substitutes for the original road occupancy system.
    pub fn speed_limit_at(&self, pos: Vec3) -> Option<f32> {
        let mut nearest = None;
        let mut distance = 20.0f32.powi(2);
        for i in self.links_near(pos, 20.0) {
            let link = &self.links[i];
            let q = link.point_at(link.project(pos));
            // Do not borrow the speed of a freeway above/below a surface road.
            if (q.y - pos.y).abs() > 4.0 {
                continue;
            }
            let d = q.distance_squared(pos);
            if d < distance {
                distance = d;
                nearest = Some(i);
            }
        }
        nearest.and_then(|i| self.speed_limit_mps(i))
    }

    /// Nearest link (by distance to the segment) and the parameter along it.
    pub fn nearest_link(&self, p: Vec3) -> Option<(usize, f32, f32)> {
        let mut best: Option<(usize, f32, f32)> = None;
        for i in self.links_near(p, 80.0) {
            let l = &self.links[i];
            let t = l.project(p);
            let q = l.point_at(t);
            let d = Vec3::new(q.x - p.x, (q.y - p.y) * 0.3, q.z - p.z).length();
            if best.map_or(true, |b| d < b.2) {
                best = Some((i, t, d));
            }
        }
        if best.is_none() {
            // fall back to brute force
            for (i, l) in self.links.iter().enumerate() {
                let t = l.project(p);
                let d = l.point_at(t).distance(p);
                if best.map_or(true, |b| d < b.2) {
                    best = Some((i, t, d));
                }
            }
        }
        best
    }

    /// Nearest link preferring ones aligned with `heading`.
    pub fn nearest_link_aligned(&self, p: Vec3, heading: Vec3) -> Option<(usize, f32)> {
        let mut best: Option<(usize, f32, f32)> = None;
        for i in self.links_near(p, 80.0) {
            let l = &self.links[i];
            let t = l.project(p);
            let q = l.point_at(t);
            let d = Vec3::new(q.x - p.x, (q.y - p.y) * 0.3, q.z - p.z).length();
            let align = l.dir().dot(heading); // -1..1
            let score = d + (1.0 - align) * 9.0;
            if best.map_or(true, |b| score < b.2) {
                best = Some((i, t, score));
            }
        }
        best.map(|(i, t, _)| (i, t))
    }

    fn sccs(&self) -> &Sccs {
        self.sccs.get_or_init(|| {
            // Tarjan's, without recursion: (link, next edge to try) on `call`
            let n = self.links.len();
            let edge = |u: usize, k: usize| -> Option<usize> {
                let l = &self.links[u];
                if k < l.next.len() {
                    Some(l.next[k])
                } else {
                    l.side.get(k - l.next.len()).copied()
                }
            };
            let (mut index, mut low, mut on) = (vec![u32::MAX; n], vec![0u32; n], vec![false; n]);
            let mut comp = vec![u32::MAX; n];
            let (mut stack, mut call): (Vec<usize>, Vec<(usize, usize)>) = (Vec::new(), Vec::new());
            let (mut idx, mut nc) = (0u32, 0u32);
            for s in 0..n {
                if index[s] != u32::MAX {
                    continue;
                }
                index[s] = idx;
                low[s] = idx;
                idx += 1;
                stack.push(s);
                on[s] = true;
                call.push((s, 0));
                while let Some(&(u, k)) = call.last() {
                    if let Some(v) = edge(u, k) {
                        call.last_mut().unwrap().1 += 1;
                        if index[v] == u32::MAX {
                            index[v] = idx;
                            low[v] = idx;
                            idx += 1;
                            stack.push(v);
                            on[v] = true;
                            call.push((v, 0));
                        } else if on[v] {
                            low[u] = low[u].min(index[v]);
                        }
                    } else {
                        call.pop();
                        if let Some(&(p, _)) = call.last() {
                            low[p] = low[p].min(low[u]);
                        }
                        if low[u] == index[u] {
                            while let Some(w) = stack.pop() {
                                on[w] = false;
                                comp[w] = nc;
                                if w == u {
                                    break;
                                }
                            }
                            nc += 1;
                        }
                    }
                }
            }
            let mut succ = vec![Vec::new(); nc as usize];
            for u in 0..n {
                let mut k = 0;
                while let Some(v) = edge(u, k) {
                    if comp[v] != comp[u] {
                        succ[comp[u] as usize].push(comp[v]);
                    }
                    k += 1;
                }
            }
            for s in succ.iter_mut() {
                s.sort_unstable();
                s.dedup();
            }
            Sccs { comp, succ }
        })
    }

    /// Can `path_to_point(from, goal, radius)` find anything: is a link it would stop on (passing within `radius`
    /// of `goal`, 8 m in height) reachable from `from`? Exact: the links near the goal are taken from the grid
    /// out to the longest link's length, then the components' graph is searched from `from`'s.
    fn goal_reachable(&self, from: usize, goal: Vec3, radius: f32) -> bool {
        let goals: Vec<usize> = self
            .links_near(goal, radius + self.max_link_len)
            .filter(|&i| {
                let l = &self.links[i];
                let q = l.point_at(l.project(goal));
                xz(q, goal) < radius && (q.y - goal.y).abs() < 8.0
            })
            .collect();
        if goals.is_empty() {
            return false;
        }
        let sc = self.sccs();
        let mut want = vec![false; sc.succ.len()];
        for &g in &goals {
            want[sc.comp[g] as usize] = true;
        }
        let mut seen = vec![false; sc.succ.len()];
        let mut todo = vec![sc.comp[from]];
        seen[sc.comp[from] as usize] = true;
        while let Some(c) = todo.pop() {
            if want[c as usize] {
                return true;
            }
            for &d in &sc.succ[c as usize] {
                if !seen[d as usize] {
                    seen[d as usize] = true;
                    todo.push(d);
                }
            }
        }
        false
    }

    /// Shortest path (by length) from link `from` to any link passing within `radius`
    /// of `goal` (A* over the directed lane graph). Returns the links, both ends inclusive.
    pub fn path_to_point(&self, from: usize, goal: Vec3, radius: f32) -> Option<Vec<usize>> {
        // (none to be found: answered without walking the graph - the search would return None after every link
        // reachable from `from`)
        if !self.goal_reachable(from, goal, radius) {
            return None;
        }
        self.search(from, goal, radius)
    }

    /// `path_to_point`'s search itself.
    fn search(&self, from: usize, goal: Vec3, radius: f32) -> Option<Vec<usize>> {
        use std::cmp::Ordering;
        use std::collections::BinaryHeap;
        #[derive(PartialEq)]
        struct N(f32, usize);
        impl Eq for N {}
        impl Ord for N {
            fn cmp(&self, o: &Self) -> Ordering {
                o.0.partial_cmp(&self.0).unwrap_or(Ordering::Equal)
            }
        }
        impl PartialOrd for N {
            fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
                Some(self.cmp(o))
            }
        }
        let n = self.links.len();
        let is_goal = |i: usize| {
            let l = &self.links[i];
            let q = l.point_at(l.project(goal));
            xz(q, goal) < radius && (q.y - goal.y).abs() < 8.0
        };
        let mut dist = vec![f32::INFINITY; n];
        let mut prev = vec![usize::MAX; n];
        let mut heap = BinaryHeap::new();
        dist[from] = 0.0;
        heap.push(N(self.links[from].b.distance(goal), from));
        while let Some(N(_, u)) = heap.pop() {
            if is_goal(u) {
                let mut out = vec![u];
                let mut c = u;
                while prev[c] != usize::MAX {
                    c = prev[c];
                    out.push(c);
                }
                out.reverse();
                return Some(out);
            }
            let du = dist[u];
            for &v in &self.links[u].next {
                let nd = du + self.links[v].len;
                if nd < dist[v] {
                    dist[v] = nd;
                    prev[v] = u;
                    heap.push(N(nd + self.links[v].b.distance(goal), v));
                }
            }
            for &v in &self.links[u].side {
                // a lane change costs the rest of the neighbouring link plus a small penalty
                let t = self.links[v].project(self.links[u].b);
                let nd = du + self.links[v].len * (1.0 - t) + 6.0;
                if nd < dist[v] {
                    dist[v] = nd;
                    prev[v] = u;
                    heap.push(N(nd + self.links[v].b.distance(goal), v));
                }
            }
        }
        None
    }

    /// Shortest path between two links.
    pub fn path(&self, from: usize, to: usize) -> Option<Vec<usize>> {
        let l = &self.links[to];
        let mid = (l.a + l.b) * 0.5;
        let p = self.path_to_point(from, mid, 0.6)?;
        if p.last() == Some(&to) {
            Some(p)
        } else {
            None
        }
    }

    /// Driving route between two world positions as a polyline along the lanes.
    /// `heading` selects the lane to start on. The polyline ends near `to`.
    pub fn route_points(&self, from: Vec3, heading: Option<Vec3>, to: Vec3) -> Option<Vec<Vec3>> {
        // candidate start links: the best aligned one first, then other close lanes
        let mut starts: Vec<usize> = Vec::new();
        if let Some(h) = heading {
            if let Some((l, _)) = self.nearest_link_aligned(from, h) {
                starts.push(l);
            }
        }
        let mut near: Vec<(f32, usize)> = self
            .links_near(from, 30.0)
            .filter_map(|i| {
                let l = &self.links[i];
                let q = l.point_at(l.project(from));
                let d = xz(q, from);
                if d < 30.0 && (q.y - from.y).abs() < 8.0 {
                    let pen = heading.map_or(0.0, |h| (1.0 - l.dir().dot(h)) * 12.0);
                    Some((d + pen, i))
                } else {
                    None
                }
            })
            .collect();
        near.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        for (_, i) in near.into_iter().take(8) {
            if !starts.contains(&i) {
                starts.push(i);
            }
        }
        for radius in [9.0f32, 18.0, 32.0] {
            for &s in &starts {
                if let Some(p) = self.path_to_point(s, to, radius) {
                    let mut pts: Vec<Vec3> = Vec::with_capacity(p.len() + 2);
                    let t0 = self.links[s].project(from);
                    pts.push(self.links[s].point_at(t0));
                    for (k, &li) in p.iter().enumerate() {
                        let l = &self.links[li];
                        if k + 1 == p.len() {
                            // stop at the point of the last link closest to the goal
                            let t = l.project(to);
                            let t = if p.len() == 1 { t.max(t0) } else { t };
                            pts.push(l.point_at(t));
                        } else {
                            pts.push(l.b);
                        }
                    }
                    return Some(pts);
                }
            }
        }
        None
    }
    /// Race line: the shortest route from `start` (facing `heading`) through all `cps`
    /// in order, passing within `radius` of each.
    ///
    /// Racers are not bound by traffic rules, so the search runs over (link, direction)
    /// states: lanes may be driven against traffic and oncoming lanes may be crossed, at
    /// a cost, which keeps the line on the correct side wherever that is not much
    /// longer. All legs are solved jointly (multi-source Dijkstra per leg over the
    /// candidate states at each checkpoint). Returns the polyline and the number of legs
    /// that had no road route (bridged with a straight line).
    pub fn route_through(&self, start: Vec3, heading: Vec3, cps: &[Vec3], radius: f32) -> (Vec<Vec3>, usize) {
        use std::cmp::Ordering;
        use std::collections::BinaryHeap;
        #[derive(PartialEq)]
        struct N(f32, usize);
        impl Eq for N {}
        impl Ord for N {
            fn cmp(&self, o: &Self) -> Ordering {
                o.0.partial_cmp(&self.0).unwrap_or(Ordering::Equal)
            }
        }
        impl PartialOrd for N {
            fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
                Some(self.cmp(o))
            }
        }
        const TELEPORT: u32 = u32::MAX - 1;
        const WRONG: f32 = 1.5; // cost factor for driving against traffic
        let n = self.links.len();
        let ns = n * 2; // state = link * 2 + (1 if reversed)
        let mut pts = vec![start];
        if n == 0 || cps.is_empty() {
            pts.extend_from_slice(cps);
            return (pts, cps.len());
        }
        let cands_at = |p: Vec3| -> Vec<usize> {
            let mut v: Vec<usize> = Vec::new();
            for i in self.links_near(p, radius + 10.0) {
                let l = &self.links[i];
                let q = l.point_at(l.project(p));
                if xz(q, p) < radius && (q.y - p.y).abs() < 8.0 {
                    v.push(i * 2);
                    v.push(i * 2 + 1);
                }
            }
            if v.is_empty() {
                if let Some((i, _, _)) = self.nearest_link(p) {
                    v.push(i * 2);
                    v.push(i * 2 + 1);
                }
            }
            v
        };
        // start states: lanes around the start, entered in the direction we are facing
        let mut sources: Vec<(usize, f32)> = Vec::new();
        {
            // lanes around the start; when the start is off the lanes (car park, plaza)
            // widen the search to the closest ones
            let mut near: Vec<(f32, usize, f32)> = Vec::new();
            for i in self.links_near(start, 60.0) {
                let l = &self.links[i];
                let t = l.project(start);
                let q = l.point_at(t);
                let d = xz(q, start);
                if d < 60.0 && (q.y - start.y).abs() < 6.0 {
                    near.push((d, i, t));
                }
            }
            let dmin = near.iter().map(|x| x.0).fold(f32::INFINITY, f32::min);
            let reach = (dmin + 8.0).max(14.0);
            for (d, i, t) in near {
                if d > reach {
                    continue;
                }
                let l = &self.links[i];
                let dot = l.dir().dot(heading);
                // with traffic
                sources.push((i * 2, d * 2.0 + (1.0 - dot) * 12.0 + l.len * (1.0 - t)));
                // against traffic
                sources.push((i * 2 + 1, d * 2.0 + (1.0 + dot) * 12.0 + 10.0 + l.len * t * WRONG));
            }
        }
        if sources.is_empty() {
            let s0 = self.nearest_link_aligned(start, heading).map(|x| x.0).or_else(|| self.nearest_link(start).map(|x| x.0)).unwrap_or(0);
            sources.push((s0 * 2, 0.0));
        }
        let first_sources: Vec<usize> = sources.iter().map(|x| x.0).collect();
        let mut leg_prev: Vec<Vec<u32>> = Vec::with_capacity(cps.len());
        let mut leg_cands: Vec<Vec<(usize, f32)>> = Vec::with_capacity(cps.len());
        let mut missing = 0usize;
        let mut dist = vec![f32::INFINITY; ns];
        for cp in cps {
            let cands = cands_at(*cp);
            let mut is_cand = vec![false; ns];
            for &c in &cands {
                is_cand[c] = true;
            }
            for d in dist.iter_mut() {
                *d = f32::INFINITY;
            }
            let mut prev = vec![u32::MAX; ns];
            let mut heap = BinaryHeap::new();
            for &(st, c) in &sources {
                if c < dist[st] {
                    dist[st] = c;
                    heap.push(N(c, st));
                }
            }
            let mut settled = 0usize;
            let mut best_c = f32::INFINITY;
            while let Some(N(d, u)) = heap.pop() {
                if d > dist[u] {
                    continue;
                }
                if is_cand[u] {
                    settled += 1;
                    best_c = best_c.min(d);
                    if settled >= cands.len() {
                        break;
                    }
                }
                if d > best_c + 300.0 {
                    break;
                }
                let li = u / 2;
                let lu = &self.links[li];
                let mut relax = |v: usize, nd: f32, dist: &mut Vec<f32>, prev: &mut Vec<u32>, heap: &mut BinaryHeap<N>| {
                    if nd < dist[v] {
                        dist[v] = nd;
                        prev[v] = u as u32;
                        heap.push(N(nd, v));
                    }
                };
                if u % 2 == 0 {
                    // travelling with traffic, now at lu.b
                    for &k in &lu.next {
                        relax(k * 2, d + self.links[k].len, &mut dist, &mut prev, &mut heap);
                    }
                    for &k in &self.ends_at[lu.node_b] {
                        if k != li {
                            relax(k * 2 + 1, d + self.links[k].len * WRONG + 8.0, &mut dist, &mut prev, &mut heap);
                        }
                    }
                    for &k in &lu.side {
                        let t = self.links[k].project(lu.b);
                        relax(k * 2, d + self.links[k].len * (1.0 - t) + 6.0, &mut dist, &mut prev, &mut heap);
                    }
                    for &k in &lu.opp {
                        let t = self.links[k].project(lu.b);
                        relax(k * 2 + 1, d + self.links[k].len * t * WRONG + 12.0, &mut dist, &mut prev, &mut heap);
                    }
                    for &k in &lu.hop {
                        let gap = self.links[k].a.distance(lu.b);
                        relax(k * 2, d + gap * 1.7 + 15.0 + self.links[k].len, &mut dist, &mut prev, &mut heap);
                    }
                } else {
                    // travelling against traffic, now at lu.a
                    for &k in &self.starts_at[lu.node_a] {
                        if k != li {
                            relax(k * 2, d + self.links[k].len, &mut dist, &mut prev, &mut heap);
                        }
                    }
                    for &k in &lu.prev {
                        relax(k * 2 + 1, d + self.links[k].len * WRONG, &mut dist, &mut prev, &mut heap);
                    }
                    for &k in &lu.back {
                        let t = self.links[k].project(lu.a);
                        relax(k * 2, d + self.links[k].len * (1.0 - t) + 6.0, &mut dist, &mut prev, &mut heap);
                    }
                    for &k in &lu.hop_a {
                        let gap = self.links[k].a.distance(lu.a);
                        relax(k * 2, d + gap * 1.7 + 15.0 + self.links[k].len, &mut dist, &mut prev, &mut heap);
                    }
                }
            }
            let mut reached: Vec<(usize, f32)> = cands.iter().filter(|&&c| dist[c].is_finite()).map(|&c| (c, dist[c])).collect();
            if reached.is_empty() {
                // unreachable by road: jump to the checkpoint and continue from there
                missing += 1;
                let base = sources.iter().map(|x| x.1).fold(f32::INFINITY, f32::min);
                reached = cands.iter().map(|&c| (c, base + 5000.0)).collect();
                for &c in &cands {
                    prev[c] = TELEPORT;
                }
            }
            leg_prev.push(prev);
            leg_cands.push(reached.clone());
            sources = reached;
        }
        // backtrack from the cheapest final candidate
        let mut chain: Vec<usize> = Vec::new();
        let mut cur = sources.iter().min_by(|a, b| a.1.partial_cmp(&b.1).unwrap()).map(|x| x.0).unwrap_or(first_sources[0]);
        for k in (0..cps.len()).rev() {
            let prev = &leg_prev[k];
            let mut seg = vec![cur];
            let mut guard = 0;
            while prev[cur] != u32::MAX && prev[cur] != TELEPORT && guard < ns {
                cur = prev[cur] as usize;
                seg.push(cur);
                guard += 1;
            }
            if prev[cur] == TELEPORT && k > 0 {
                cur = leg_cands[k - 1].iter().min_by(|a, b| a.1.partial_cmp(&b.1).unwrap()).map(|x| x.0).unwrap_or(cur);
                seg.push(cur);
            }
            // cur is now the source state of this leg, i.e. a candidate of the previous
            // leg; that leg emits it, so drop the duplicate here
            if k > 0 {
                seg.pop();
            }
            chain.extend(seg);
        }
        chain.reverse();
        for (k, &st) in chain.iter().enumerate() {
            let l = &self.links[st / 2];
            let (entry, exit) = if st % 2 == 0 { (l.a, l.b) } else { (l.b, l.a) };
            if k == 0 {
                let t0 = l.project(start);
                let q = l.point_at(t0);
                if q.distance(start) > 1.0 {
                    pts.push(q);
                }
            } else if pts.last().map_or(false, |q| q.distance(entry) > 9.0) {
                // junction hop: drive to the start of the lane we hop onto
                pts.push(entry);
            }
            if pts.last().map_or(true, |q| q.distance(exit) > 0.5) {
                pts.push(exit);
            }
        }
        (pts, missing)
    }
}

#[cfg(test)]
mod speed_tests {
    use super::*;

    fn fixture() -> Resource {
        let mut data = vec![0u8; 0x700];
        let put = |d: &mut [u8], o: usize, v: u32| d[o..o + 4].copy_from_slice(&v.to_be_bytes());
        put(&mut data, 0x2c, 0x50000500);
        for (i, mph) in [30.0f32, 65.0].into_iter().enumerate() {
            let road = 0x100 + i * 0xb0;
            let lane = 0x300 + i * 0x70;
            let y = i as f32 * 10.0;
            data[road..road + 4].copy_from_slice(&ZONE_VT.to_le_bytes());
            put(&mut data, road + 0x94, (mph * 0.44704).to_bits());
            data[lane..lane + 4].copy_from_slice(&LINK_VT.to_le_bytes());
            put(&mut data, lane + 8, 0x50000000 + road as u32);
            put(&mut data, lane + 0xc, 40.0f32.to_bits());
            put(&mut data, lane + 0x14, y.to_bits());
            put(&mut data, lane + 0x20, 40.0f32.to_bits());
            put(&mut data, lane + 0x24, y.to_bits());
            data[lane + 0x4c..lane + 0x4e].copy_from_slice(&1u16.to_be_bytes());
            put(&mut data, lane + 0x50, 0x50000600 + (i * 4) as u32);
            for j in 0..2 {
                let v = i * 2 + j;
                data[0x600 + v * 2..0x602 + v * 2].copy_from_slice(&(v as u16).to_be_bytes());
                put(&mut data, 0x500 + v * 16, (j as f32 * 40.0).to_bits());
                put(&mut data, 0x504 + v * 16, y.to_bits());
            }
        }
        Resource { rtype: 66, flags: 0, virtual_size: data.len(), physical_size: 0, data }
    }

    #[test]
    fn parses_big_endian_road_speeds_and_selects_correct_overpass_level() {
        let nav = NavSys::parse(&fixture()).unwrap();
        assert_eq!(nav.zones.len(), 2);
        for (y, mph) in [(0.5, 30.0), (10.5, 65.0)] {
            let speed = nav.speed_limit_at(Vec3::new(20.0, y, 0.0)).unwrap();
            assert!((speed / 0.44704 - mph).abs() < 0.0001);
        }
        assert_eq!(nav.speed_limit_at(Vec3::new(20.0, 30.0, 0.0)), None);
        assert_eq!(nav.speed_limit_at(Vec3::new(20.0, 0.0, 100.0)), None);
    }

    #[test]
    fn missing_or_zero_speed_uses_default_but_junction_retains_previous_limit() {
        let mut res = fixture();
        res.data[0x194..0x198].fill(0);
        let mut nav = NavSys::parse(&res).unwrap();
        let p = Vec3::new(20.0, 0.5, 0.0);
        assert_eq!(nav.speed_limit_at(p), Some(DEFAULT_ROAD_SPEED));
        for link in &mut nav.links {
            link.zone = None;
        }
        assert_eq!(nav.speed_limit_at(p), None);
        assert_eq!(nav.speed_limit_mps(usize::MAX), None);
        for value in [f32::NAN, f32::INFINITY, -1.0] {
            assert_eq!(Zone { speed_limit: value, ..Default::default() }.speed_limit_mps(), DEFAULT_ROAD_SPEED);
        }
    }

    #[test]
    #[ignore = "requires CSV from tools/road_speed_reference.py; set MCLA_SPEED_REFERENCE"]
    fn matches_original_recompiled_accessor_bit_for_bit() {
        let path = std::env::var("MCLA_SPEED_REFERENCE").expect("set MCLA_SPEED_REFERENCE to the reference CSV");
        let csv = std::fs::read_to_string(path).unwrap();
        let mut count = 0;
        for line in csv.lines().skip(1) {
            let fields: Vec<_> = line.split(',').collect();
            let input = f32::from_bits(fields[1].parse().unwrap());
            let expected: u32 = fields[2].parse().unwrap();
            let actual = Zone { speed_limit: input, ..Default::default() }.speed_limit_mps();
            assert_eq!(actual.to_bits(), expected, "road {} input {input}", fields[0]);
            count += 1;
        }
        assert_eq!(count, 1664, "Complete Edition roads plus the default case");
    }

    #[test]
    #[ignore = "requires user's extracted Complete Edition data; set MCLA_DATA_DIR"]
    fn complete_edition_road_speed_census() {
        let root = std::env::var("MCLA_DATA_DIR").expect("set MCLA_DATA_DIR to extracted/cache");
        let bytes = std::fs::read(std::path::Path::new(&root).join("resources/city/sc/0x9960fcd8.r66")).unwrap();
        assert_eq!(&bytes[..4], b"MCLR");
        let data = bytes[16..].to_vec();
        let virtual_size = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let res = Resource { rtype: 66, flags: 0, virtual_size, physical_size: data.len() - virtual_size, data };
        let nav = NavSys::parse(&res).unwrap();
        let mut census = std::collections::BTreeMap::new();
        for zone in &nav.zones {
            *census.entry((zone.speed_limit_mps() / 0.44704).round() as u32).or_insert(0usize) += 1;
        }
        assert_eq!(census.into_iter().collect::<Vec<_>>(), vec![(10, 1), (15, 3), (25, 1), (30, 133), (40, 646), (45, 356), (55, 37), (60, 2), (65, 484)]);
        assert!(nav.links.iter().enumerate().any(|(i, _)| nav.speed_limit_mps(i).is_some_and(|s| s > 29.0)));
    }

    #[test]
    #[ignore = "requires user's extracted Complete Edition data; set MCLA_DATA_DIR"]
    fn complete_edition_lane_owner_flags() {
        let root = std::env::var("MCLA_DATA_DIR").expect("set MCLA_DATA_DIR to extracted/cache");
        let bytes = std::fs::read(std::path::Path::new(&root).join("resources/city/sc/0x9960fcd8.r66")).unwrap();
        let data = bytes[16..].to_vec();
        let virtual_size = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let res = Resource { rtype: 66, flags: 0, virtual_size, physical_size: data.len() - virtual_size, data };
        let nav = NavSys::parse(&res).unwrap();
        let mut census = std::collections::BTreeMap::new();
        for l in &nav.links {
            *census.entry((l.zone.is_some(), l.owner_flags, (l.owner_w10 != 0) as u8)).or_insert(0usize) += 1;
        }
        // roads 0x2, intersections 0x4, the junctions' connector lanes 0x10 / 0x20
        assert_eq!(census.into_iter().collect::<Vec<_>>(), vec![((false, 4, 1), 5060), ((false, 16, 0), 2693), ((false, 16, 1), 928), ((false, 32, 0), 1981), ((false, 32, 1), 1117), ((true, 2, 1), 23480)]);
        // the roads' rectangles for the traffic spawner: unit directions, density indices
        // -1 (the default) / 2 / 3 / 5
        let odd: Vec<_> = nav.zones.iter().filter(|z| !((z.dir.length() - 1.0).abs() < 0.01 && z.width > 0.0 && z.rect_len > 0.0)).map(|z| (z.dir, z.width, z.rect_len, z.mid)).collect();
        let mut parents = std::collections::BTreeMap::new();
        for l in &nav.links {
            *parents.entry((l.owner_flags, l.parent_flags)).or_insert(0usize) += 1;
        }
        assert_eq!(parents.into_iter().collect::<Vec<_>>(), vec![((2, 128), 23480), ((4, 128), 5060), ((16, 2), 110), ((16, 4), 3511), ((32, 2), 1239), ((32, 4), 1859)]);
        assert!(nav.links.iter().all(|l| (l.parent_flags == 2) == l.parent_zone.is_some()));
        // a lane's junction (sub_822452F8) for exactly the lanes flagged so, and every junction's
        // walked rails (sub_8273C1B8) are the lanes whose junction it is
        assert!(nav.links.iter().all(|l| ((l.owner_flags | l.parent_flags) & 4 != 0) == nav.lane_junction[l.lane].is_some()));
        assert_eq!(nav.junction_centers.len(), 351);
        for (j, ls) in nav.junction_lanes.iter().enumerate() {
            let mut a = ls.clone();
            a.sort();
            let b: Vec<usize> = (0..nav.lanes).filter(|&l| nav.lane_junction[l] == Some(j)).collect();
            assert_eq!(a, b);
        }
        assert_eq!(nav.junction_lanes.iter().filter(|l| !l.is_empty()).count(), 339);
        // the quadtree holds every lane once
        let mut seen = vec![0u8; nav.lanes];
        for q in &nav.quad {
            for &l in &q.lanes {
                seen[l] += 1;
            }
        }
        assert!(seen.iter().all(|&c| c == 1));
        // the quadtree's answer is every lane with flags & 1 whose box is within w (3D), in the
        // tree's order; the cap keeps its head
        let mut most = 0;
        for k in [0usize, 997, 2500, 4999, 7777, 10000] {
            let p = nav.curves[k].pts[0] + Vec3::new(1.5, 0.5, -2.0);
            for w in [6.0f32, 30.0, 120.0] {
                let all = nav.lanes_near(p, w, 1, usize::MAX);
                let mut a = all.clone();
                a.sort();
                let b: Vec<usize> = (0..nav.lanes).filter(|&l| nav.raw[l].flags & 1 != 0 && w * w > (nav.raw[l].min - p).max(p - nav.raw[l].max).max(Vec3::ZERO).length_squared()).collect();
                assert_eq!(a, b, "{p} {w}");
                let head = nav.lanes_near(p, w, 1, 48);
                assert_eq!(&all[..all.len().min(48)], &head[..]);
                most = most.max(all.len());
            }
        }
        assert!(most > 48, "{most}");
        // the lanes' own flags (+4): 1, or 0x2001 on some road lanes and road connectors - none
        // carries 0x1800, the bits the traffic's car-ahead search reads off run-time rails
        let mut lf = std::collections::BTreeMap::new();
        for l in &nav.links {
            *lf.entry((l.owner_flags, l.flags)).or_insert(0usize) += 1;
        }
        assert_eq!(lf.into_iter().collect::<Vec<_>>(), vec![((2, 1), 22075), ((2, 0x2001), 1405), ((4, 1), 5060), ((16, 1), 3511), ((16, 0x2001), 110), ((32, 1), 3098)]);
        // (43 roads carry no rectangle at all)
        assert!(odd.len() == 43 && odd.iter().all(|o| o.1 == 0.0 && o.2 == 0.0), "{} odd: {:?}", odd.len(), &odd[..odd.len().min(5)]);
        let mut dens = std::collections::BTreeMap::new();
        for z in &nav.zones {
            *dens.entry(z.density).or_insert(0usize) += 1;
        }
        assert_eq!(dens.into_iter().collect::<Vec<_>>(), vec![(-1, 756), (2, 140), (3, 516), (5, 251)]);
        assert_eq!(parse_intersections(&res).len(), 351);
        let sides = nav.zones.iter().filter(|z| z.rect_len > 0.0 && ((z.side.length() - 1.0).abs() > 0.01 || z.side.dot(z.dir).abs() > 0.01)).count();
        let ys = nav.zones.iter().filter(|z| z.rect_len > 0.0 && !(z.y_range[0] <= z.y_range[1])).count();
        // (the 43 without a rectangle are the ones flagged to have no box)
        assert_eq!((sides, ys, nav.zones.iter().filter(|z| z.no_box).count(), nav.zones.iter().filter(|z| z.no_box && z.rect_len == 0.0).count()), (0, 0, 43, 43));
    }

    #[test]
    #[ignore = "requires user's extracted Complete Edition data; set MCLA_DATA_DIR"]
    fn complete_edition_lane_curves() {
        let root = std::env::var("MCLA_DATA_DIR").expect("set MCLA_DATA_DIR to extracted/cache");
        let bytes = std::fs::read(std::path::Path::new(&root).join("resources/city/sc/0x9960fcd8.r66")).unwrap();
        let data = bytes[16..].to_vec();
        let virtual_size = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let res = Resource { rtype: 66, flags: 0, virtual_size, physical_size: data.len() - virtual_size, data };
        let nav = NavSys::parse(&res).unwrap();
        let mut deg = std::collections::BTreeMap::new();
        for c in &nav.curves {
            *deg.entry(c.degree).or_insert(0usize) += 1;
        }
        assert_eq!(deg.into_iter().collect::<Vec<_>>(), vec![(1, 1106), (2, 4291), (3, 4634)]);
        // every curve's length as sub_82244E10 measures it is the stored one (+0c)
        let worst = nav.raw.iter().map(|l| (l.curve.length() - l.len).abs()).fold(0.0f32, f32::max);
        assert!(worst < 0.05, "worst length error {worst}");
        // every lane's last control point is another lane's first
        assert!(nav.raw.iter().all(|l| nav.raw.iter().any(|m| m.vi.0 == l.vi.1)));
        // the junction lane seen turning a car 90 degrees at its middle vertex (2026-10-20):
        // a quadratic arc of 29.45 m from (-604.3, -641.7) to (-622.9, -623.9)
        let c = nav.curves.iter().find(|c| c.pts.len() == 3 && c.pts[1].distance(Vec3::new(-622.7, 17.8, -641.6)) < 0.05).unwrap();
        assert_eq!(c.degree, 2);
        assert!((c.length() - 29.45).abs() < 0.02, "{}", c.length());
        let (mid, d) = c.eval(0.5);
        assert!(mid.distance(Vec3::new(-618.15, 0.0, -637.2) + Vec3::Y * mid.y) < 0.1, "{mid:?}");
        assert!((d.normalize().x + d.normalize().z).abs() < 0.05, "{d:?}");
        // the curved graph keeps every lane and its successors; its tangents turn under 3
        // degrees a link (2 and, at a lane's end, one trial step more) but at the cusps of
        // curves with coincident control points (58 links over 0.25 m of 172905)
        let cv = nav.curved(2f32.to_radians());
        assert_eq!(cv.lanes, nav.lanes);
        let turns = cv.links.iter().filter(|l| l.len > 0.25 && l.ta.dot(l.tb) < 3f32.to_radians().cos()).count();
        eprintln!("links: control polygon {}, curved {}", nav.links.len(), cv.links.len());
        assert!(turns < 100, "{turns}");
        // each lane's links carry its phases in order, and the curve at them is the link's ends
        let mut bad = Vec::new();
        for (k, l) in cv.links.iter().enumerate() {
            let c = &cv.curves[l.lane];
            let same_next = k + 1 < cv.links.len() && cv.links[k + 1].lane == l.lane;
            if !(l.t[0] < l.t[1]) || (same_next && (cv.links[k + 1].t[0] - l.t[1]).abs() > 1e-6) || c.eval(l.t[0]).0.distance(l.a) > 0.01 || (l.t[1] < 1.0 && c.eval(l.t[1]).0.distance(l.b) > 0.01) {
                bad.push((k, l.lane, l.t, l.a, l.b, c.eval(l.t[0]).0));
            }
        }
        assert!(bad.is_empty(), "{} bad links, e.g. {:?}", bad.len(), &bad[..bad.len().min(4)]);
        let ends = |g: &NavSys| (0..g.links.len()).filter(|&i| i + 1 == g.links.len() || g.links[i + 1].lane != g.links[i].lane).map(|i| { let mut v: Vec<usize> = g.links[i].next.iter().map(|&k| g.links[k].lane).collect(); v.sort(); v }).collect::<Vec<_>>();
        assert_eq!(ends(&cv), ends(&nav));
    }
}

/// A signalled junction of the road network (file class 0x47a0c4, the game's
/// `mcRnIntersectionAmbient`): root +58 -> array of 0xc0-byte objects, count at root +c8.
/// +2c -> the arms' outward unit directions (vec4 each), +30 u16 their number; +3c / +40 the
/// box's half extents; +80 the centre (y = street level). Layout and use read from the
/// executable: work/map/signal_rule.md.
#[derive(Debug, Clone)]
pub struct NavJunction {
    pub center: glam::Vec3,
    pub arms: Vec<glam::Vec3>,
    pub half: [f32; 2],
}

/// Every junction as the traffic spawner weighs it (`mcRnIntersectionAmbient`): centre (+80),
/// its weight (+1c: lane length, times 0.01 x the density value) and its reach (+40).
pub fn parse_intersections(res: &crate::rsc::Resource) -> Vec<(glam::Vec3, f32, f32)> {
    let rd = super::Rd::new(res);
    let mut out = Vec::new();
    let Some(arr) = rd.ptr(0x58) else { return out };
    let n = (rd.u32(0xc8) as usize).min(4096);
    for i in 0..n {
        let o = arr + i * 0xc0;
        if !rd.has(o, 0xc0) || rd.vtable(o) != 0x0047_a0c4 {
            continue;
        }
        let c = glam::Vec3::new(rd.f32(o + 0x80), rd.f32(o + 0x84), rd.f32(o + 0x88));
        if c.is_finite() {
            out.push((c, rd.f32(o + 0x1c), rd.f32(o + 0x40)));
        }
    }
    out
}

/// A region's rectangle and box as the ambient spawners test them (the region base shared by
/// roads and junctions: +34 / +38 the sphere test's x / z, +3c width, +40 length, +44 / +48 the
/// height range, +50 centre, +60 direction, +70 across; sub_8224D4C0, sub_826F4E90).
#[derive(Debug, Clone, Copy, Default)]
pub struct RegionRect {
    /// the junction's index in the file's array (as `NavSys::junction_lanes`)
    pub index: usize,
    pub rect_c: [f32; 2],
    pub width: f32,
    pub len: f32,
    pub y_range: [f32; 2],
    pub mid: glam::Vec3,
    pub dir: glam::Vec3,
    pub side: glam::Vec3,
}

/// The junctions' rectangles, in the order (and with the filter) of [`parse_intersections`].
pub fn parse_intersection_rects(res: &crate::rsc::Resource) -> Vec<RegionRect> {
    let rd = super::Rd::new(res);
    let mut out = Vec::new();
    let Some(arr) = rd.ptr(0x58) else { return out };
    let n = (rd.u32(0xc8) as usize).min(4096);
    let v = |p: usize| glam::Vec3::new(rd.f32(p), rd.f32(p + 4), rd.f32(p + 8));
    for i in 0..n {
        let o = arr + i * 0xc0;
        if !rd.has(o, 0xc0) || rd.vtable(o) != 0x0047_a0c4 || !v(o + 0x80).is_finite() {
            continue;
        }
        out.push(RegionRect { index: i, rect_c: [rd.f32(o + 0x34), rd.f32(o + 0x38)], width: rd.f32(o + 0x3c), len: rd.f32(o + 0x40), y_range: [rd.f32(o + 0x44), rd.f32(o + 0x48)], mid: v(o + 0x50), dir: v(o + 0x60), side: v(o + 0x70) });
    }
    out
}

/// The junctions that have arms (339 of the 351 in the file).
pub fn parse_junctions(res: &crate::rsc::Resource) -> Vec<NavJunction> {
    let rd = super::Rd::new(res);
    let mut out = Vec::new();
    let Some(arr) = rd.ptr(0x58) else { return out };
    let n = (rd.u32(0xc8) as usize).min(4096);
    for i in 0..n {
        let o = arr + i * 0xc0;
        if !rd.has(o, 0xc0) || rd.vtable(o) != 0x0047_a0c4 {
            continue;
        }
        let v = |p: usize| glam::Vec3::new(rd.f32(p), rd.f32(p + 4), rd.f32(p + 8));
        let count = rd.u16(o + 0x30) as usize;
        let Some(dirs) = rd.ptr(o + 0x2c) else { continue };
        let arms: Vec<glam::Vec3> = (0..count.min(16)).map(|k| v(dirs + k * 16)).filter(|d| d.is_finite() && d.length_squared() > 0.25).collect();
        let center = v(o + 0x80);
        if arms.is_empty() || !center.is_finite() {
            continue;
        }
        out.push(NavJunction { center, arms, half: [rd.f32(o + 0x3c), rd.f32(o + 0x40)] });
    }
    out
}

#[cfg(test)]
mod reach_tests {
    use super::*;

    /// `path_to_point`'s shortcut (`goal_reachable`) against the full search on the city's own lane graph
    /// (MCLA_DATA_DIR = extracted/cache, when present): the same answer for every query - goals on the lanes and
    /// goals 20 m above or below them, where the full search walks the graph and finds nothing - and the time.
    #[test]
    fn shortcut_matches_the_search() {
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let dir = std::path::Path::new(&root).join("resources/city/sc");
        let Some(p) = std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().map_or(false, |x| x == "r66")).max_by_key(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)) else { return };
        let d = std::fs::read(&p).unwrap();
        let res = if &d[0..4] == b"MCLR" {
            let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
            let data = d[16..].to_vec();
            let vs = (u(12) as usize).min(data.len());
            Resource { rtype: u(4), flags: u(8), virtual_size: vs, physical_size: data.len() - vs, data }
        } else {
            Resource::parse(&d).unwrap()
        };
        let nav = NavSys::parse(&res).unwrap();
        let n = nav.links.len();
        let mut x = 12345u32;
        let mut rnd = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as usize
        };
        let (mut t_full, mut t_new, mut found, mut none) = (0.0f64, 0.0f64, 0, 0);
        for q in 0..300 {
            let from = rnd() % n;
            let to = &nav.links[rnd() % n];
            let goal = (to.a + to.b) * 0.5 + Vec3::Y * [0.0, 20.0, -20.0][q % 3];
            for r in [9.0f32, 18.0, 32.0] {
                let t0 = std::time::Instant::now();
                let a = nav.search(from, goal, r);
                let t1 = std::time::Instant::now();
                let b = nav.path_to_point(from, goal, r);
                let t2 = std::time::Instant::now();
                t_full += (t1 - t0).as_secs_f64();
                t_new += (t2 - t1).as_secs_f64();
                assert_eq!(a, b, "from {from} goal {goal:?} radius {r}");
                if a.is_some() { found += 1 } else { none += 1 }
            }
        }
        eprintln!("{} links, {} components; {} found, {} none; full search {:.1} ms, with the shortcut {:.1} ms", n, nav.sccs().succ.len(), found, none, t_full * 1000.0, t_new * 1000.0);
    }
}
