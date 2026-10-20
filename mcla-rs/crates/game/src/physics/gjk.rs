//! Nearest points of a convex shape and a triangle as the original finds them (0x825E0150):
//! the Gilbert-Johnson-Keerthi loop over the shapes' cores - each shape without its margin -
//! with the margins taken off the distance afterwards. The loop is iterative, and on a round
//! shape it stops where its own tests say so, not where the true nearest points are: against
//! the edge of a polygon a wheel's distance is that to a chord between the few points of the
//! tyre the loop tried, up to 1.3 mm more than to the tyre's round. What the original's wheels
//! measure is this routine's answer, and the answer depends on the path the loop takes, so
//! this is the routine operation for operation, in single precision and in the original's
//! order of sums (decoded notes: work/veh/wheel_support_rule.md and work/veh/gjk_exact.md;
//! checked bit for bit against the original code run offline, tools/gjk_reference.py).

use glam::Vec3;

/// A sum of three products as the original's vector unit forms it.
#[inline]
fn dot(a: Vec3, b: Vec3) -> f32 {
    (0.0 + a.z * b.z) + (a.y * b.y + a.x * b.x)
}

#[inline]
fn yzx(a: Vec3) -> Vec3 {
    Vec3::new(a.y, a.z, a.x)
}

#[inline]
fn zxy(a: Vec3) -> Vec3 {
    Vec3::new(a.z, a.x, a.y)
}

/// A cross product as the original writes it (two rounded products, their difference, negated).
#[inline]
fn cross(a: Vec3, b: Vec3) -> Vec3 {
    let m = yzx(a) * zxy(b);
    -(zxy(a) * yzx(b) - m)
}

/// (for the other routines that are written out operation for operation)
pub fn cross_as_original(a: Vec3, b: Vec3) -> Vec3 {
    cross(a, b)
}

pub fn recip_as_original(x: f32) -> f32 {
    recip(x)
}

pub fn rsqrt_as_original(l2: f32) -> f32 {
    rsqrt(l2)
}

/// 1 / sqrt, the estimate and one Newton step.
#[inline]
fn rsqrt(l2: f32) -> f32 {
    let x = 1.0 / l2.sqrt();
    (-(l2 * (x * x) - 1.0)) * (0.5 * x) + x
}

/// The same where the original guards a zero length (the estimate's infinity becomes 0).
#[inline]
fn rsqrt_or_zero(l2: f32) -> f32 {
    let mut x = 1.0 / l2.sqrt();
    if x == f32::INFINITY {
        x = 0.0;
    }
    (-(l2 * (x * x) - 1.0)) * (0.5 * x) + x
}

/// 1 / x, the estimate and one Newton step.
#[inline]
fn recip(x: f32) -> f32 {
    let r = 1.0 / x;
    r * (-(r * x - 1.0)) + r
}

/// a * b + c in one rounding to double, then to single (the scalar unit's fused form).
#[inline]
fn fma(a: f32, b: f32, c: f32) -> f32 {
    (a as f64).mul_add(b as f64, c as f64) as f32
}

/// c - a * b likewise.
#[inline]
fn fnms(a: f32, b: f32, c: f32) -> f32 {
    (-(a as f64).mul_add(b as f64, -(c as f64))) as f32
}

/// The loop's small numbers, as the original's tables hold them.
const REL: f32 = f32::from_bits(0x358637be); // 1e-6 (0x828FF8D0)
const FLAT: f32 = f32::from_bits(0x358637bd); // 1e-6 (0x82081FD0, 0x827D8510)
const NEAR2: f32 = f32::from_bits(0x38d1b717); // 1e-4 (0x828FF8C0)
const CATCH: f32 = f32::from_bits(0x3c23d70a); // 0.01 (0x828FF900, 0x828126F0)
const LEAST2: f32 = f32::from_bits(0x28800000); // 1.42e-14 (0x828FF8F0)

/// A convex core: the point of it farthest along `dir` (which need not be a unit vector).
pub trait Support {
    fn support(&self, dir: Vec3) -> Vec3;
    /// The same with the shape moved by `by` and then by `then`.
    fn support_at(&self, dir: Vec3, by: Vec3, then: Vec3) -> Vec3 {
        self.support(dir) + by + then
    }
    /// The same with the number of the corner it is (a polyhedron's; 0 for a round shape).
    fn support_id(&self, dir: Vec3, by: Vec3, then: Vec3) -> (Vec3, u32) {
        (self.support_at(dir, by, then), 0)
    }
    /// (for the debug record of the routine's inputs: radius, width, axes, centre)
    fn describe(&self) -> Option<[u32; 14]> {
        None
    }
}

/// A shape moved.
pub struct Moved<'a, S: Support>(pub &'a S, pub Vec3);

impl<S: Support> Support for Moved<'_, S> {
    fn support(&self, dir: Vec3) -> Vec3 {
        self.0.support_at(dir, self.1, Vec3::ZERO)
    }
    fn support_at(&self, dir: Vec3, by: Vec3, then: Vec3) -> Vec3 {
        self.0.support_at(dir, self.1 + by, then)
    }
}

/// The corners of a polyhedron (a car's hull: its core is its stored corners).
pub struct Points<'a>(pub &'a [Vec3]);

impl Support for Points<'_> {
    fn support(&self, dir: Vec3) -> Vec3 {
        let mut best = (self.0[0], f32::NEG_INFINITY);
        for v in self.0 {
            let d = v.dot(dir);
            if d > best.1 {
                best = (*v, d);
            }
        }
        best.0
    }
}

/// A car's hull as the detector has it (a polyhedron bound, type 4: 0x825E0418): its corners in
/// the car's frame - all but the last, which the bound's count leaves out - and the body's
/// matrix. The corner farthest along a direction is looked for from the last of them, then
/// from the first on; a later one wins only by being farther.
#[derive(Clone, Copy, Debug)]
pub struct HullCore<'a> {
    pub corners: &'a [Vec3],
    pub rows: [Vec3; 3],
    pub at: Vec3,
}

impl Support for HullCore<'_> {
    fn support(&self, dir: Vec3) -> Vec3 {
        self.support_at(dir, Vec3::ZERO, Vec3::ZERO)
    }

    fn support_at(&self, dir: Vec3, by: Vec3, then: Vec3) -> Vec3 {
        self.support_id(dir, by, then).0
    }

    fn support_id(&self, dir: Vec3, by: Vec3, then: Vec3) -> (Vec3, u32) {
        let at = (self.at + by) + then;
        let [x, y, z] = self.rows;
        let d = Vec3::new(dot(x, dir), dot(y, dir), dot(z, dir));
        let n = self.corners.len();
        let mut p = self.corners[n - 1];
        let mut id = n - 1;
        let mut best = dot(d, p);
        for (i, v) in self.corners[..n - 1].iter().enumerate() {
            let t = dot(d, *v);
            if t > best {
                best = t;
                p = *v;
                id = i;
            }
        }
        (Vec3::new((at.x + z.x * p.z) + (y.x * p.y + x.x * p.x), (at.y + z.y * p.z) + (y.y * p.y + x.y * p.x), (at.z + z.z * p.z) + (y.z * p.y + x.z * p.x)), id as u32)
    }
}

/// A curved edge of the wheel's bound: an arc of a circle between two of its corners.
#[derive(Clone, Copy, Debug)]
struct Edge {
    centre: Vec3,
    normal: Vec3,
    radius: f32,
    ends: [usize; 2],
}

/// A curved face: a cap of a sphere, or a band of a torus (an arc revolved about the axle).
#[derive(Clone, Copy, Debug)]
struct Face {
    cap: bool,
    corner: usize,
    centre: Vec3,
    normal: Vec3,
    outer: f32,
    inner: f32,
    least: f32,
    edges: [usize; 4],
    count: usize,
}

/// The wheel bound as the car files hold it (0x8a43126c.r83, the same in every car): a wheel
/// one unit wide and one in radius, the axle along x.
const UNIT_CORNERS: [Vec3; 4] = [Vec3::new(0.5, 0.0, -1.0), Vec3::new(-0.5, 0.0, -1.0), Vec3::new(0.5, 0.0, 1.0), Vec3::new(-0.5, 0.0, 1.0)];
const ARC_AT: f32 = f32::from_bits(0x3fb988e2);
const CAP_AT: f32 = f32::from_bits(0x402ff9b3);

fn unit_edges() -> [Edge; 6] {
    let e = |centre: Vec3, normal: Vec3, radius: f32, ends: [usize; 2]| Edge { centre, normal, radius, ends };
    [
        e(Vec3::new(0.5, 0.0, 0.0), Vec3::X, 1.0, [0, 2]),
        e(Vec3::new(0.5, 0.0, 0.0), Vec3::X, 1.0, [2, 0]),
        e(Vec3::new(-0.5, 0.0, 0.0), Vec3::NEG_X, 1.0, [3, 1]),
        e(Vec3::new(-0.5, 0.0, 0.0), Vec3::NEG_X, 1.0, [1, 3]),
        e(Vec3::new(0.0, 0.0, ARC_AT), Vec3::NEG_Y, 2.5, [1, 0]),
        e(Vec3::new(0.0, 0.0, -ARC_AT), Vec3::NEG_Y, 2.5, [2, 3]),
    ]
}

fn unit_faces() -> [Face; 4] {
    let cap = |corner: usize, x: f32, normal: Vec3, edges: [usize; 4]| Face { cap: true, corner, centre: Vec3::new(x, 0.0, 0.0), normal, outer: f32::from_bits(0x4059999a), inner: -1.0, least: f32::from_bits(0x3f74ad4b), edges, count: 2 };
    let band = |corner: usize, normal: Vec3, edges: [usize; 4]| Face { cap: false, corner, centre: Vec3::ZERO, normal, outer: 2.5, inner: f32::from_bits(0xbfb988e1), least: f32::from_bits(0x3f7ad3e7), edges, count: 4 };
    [cap(0, -CAP_AT, Vec3::X, [0, 1, 0, 0]), cap(1, CAP_AT, Vec3::NEG_X, [2, 3, 0, 0]), band(1, Vec3::NEG_X, [3, 5, 0, 4]), band(0, Vec3::X, [1, 5, 2, 4])]
}

/// The wheel bound's own numbers: its four corners, six curved edges and four curved faces.
/// The original keeps one for every wheel and resizes it in place whenever the tyre's size is
/// set (0x8255BB80), each time from what the last resizing left - so its last digits carry
/// the sizes it has had before.
#[derive(Clone, Copy, Debug)]
pub struct WheelShape {
    corners: [Vec3; 4],
    edges: [Edge; 6],
    faces: [Face; 4],
}

impl WheelShape {
    /// The shape as the car files hold it.
    pub fn unit() -> Self {
        WheelShape { corners: UNIT_CORNERS, edges: unit_edges(), faces: unit_faces() }
    }

    /// The unit shape given each of `sizes` (TireRadius, TireWidth) in turn.
    pub fn sized(sizes: &[(f32, f32)]) -> Self {
        let mut shape = Self::unit();
        for &(radius, width) in sizes {
            shape.resize(radius, width);
        }
        shape
    }

    /// 0x8255BB80: to a tyre of `radius` and `width` - unless it is that already to within a
    /// millimetre both ways. The factors from its first corner; the corners by plain
    /// products (0x825A06F0), then every curved edge (0x825D19A0) and face (0x825D2878).
    pub fn resize(&mut self, radius: f32, width: f32) {
        let WheelShape { corners, edges, faces } = self;
        let v = corners[0];
        let across = fma(v.z, v.z, v.y * v.y);
        let across = if across > 0.0 { across.sqrt() } else { 0.0 };
        let (want, half) = (radius - WHEEL_MARGIN, width * 0.5);
        if !((across - want).abs() > MILLIMETRE || (v.x.abs() - half).abs() > MILLIMETRE) {
            return;
        }
        let (sx, sy) = (half / v.x.abs(), want / across);
        let scale = |p: Vec3| Vec3::new(sx * p.x, p.y * sy, p.z * sy);
        for c in corners.iter_mut() {
            *c = scale(*c);
        }
        for e in edges.iter_mut() {
            e.centre = scale(e.centre);
            let (a, b) = (corners[e.ends[0]] - e.centre, corners[e.ends[1]] - e.centre);
            let c = cross(a, b);
            let l2 = fma(c.x, c.x, fma(c.z, c.z, c.y * c.y));
            if l2 > f32::from_bits(0x2b8cbccc) {
                // (the scalar unit's 1 / sqrt: the single estimate, two Newton steps in double)
                let l = l2 as f64;
                let x = ((1.0 / l.sqrt()) as f32) as f64;
                let x = (-(x * x).mul_add(l, -1.0)).mul_add(x * 0.5, x);
                let x = (-(x * x).mul_add(l, -1.0)).mul_add(x * 0.5, x);
                let mut n = c * (x as f32);
                if fma(e.normal.x, n.x, fma(e.normal.z, n.z, e.normal.y * n.y)) < 0.0 {
                    n = Vec3::ZERO - n;
                }
                e.normal = n;
            }
            let d = e.centre - corners[e.ends[0]];
            e.radius = fma(d.x, d.x, fma(d.z, d.z, d.y * d.y)).sqrt();
        }
        for f in faces.iter_mut() {
            let old = f.centre;
            f.centre = scale(old);
            let first = edges[f.edges[0]];
            if f.cap {
                let d = f.centre - corners[f.corner];
                f.outer = fma(d.x, d.x, fma(d.z, d.z, d.y * d.y)).sqrt();
                let k = first.radius / f.outer;
                let m = fnms(k, k, 1.0);
                f.least = if m > 0.0 { m.sqrt() } else { 0.0 };
                f.inner = 0.0;
            } else {
                let other = edges[if f.count != 2 { f.edges[2] } else { f.edges[1] }];
                let axis = other.centre - first.centre;
                let len = fma(axis.x, axis.x, fma(axis.y, axis.y, axis.z * axis.z)).sqrt();
                let axis = axis * recip(len);
                // (a direction across the axis, 0x821CE640)
                let across = if axis.x.abs() >= axis.y.abs() { Vec3::new(-axis.z, 0.0, axis.x) } else { Vec3::new(0.0, axis.z, -axis.y) };
                let across = across * rsqrt_or_zero(dot(across, across));
                let a = across * f.inner + old;
                let b = across * first.radius + first.centre;
                let d = Vec3::new(a.x * sx - b.x, a.y * sy - b.y, a.z * sy - b.z);
                f.outer = fma(d.x, d.x, fma(d.z, d.z, d.y * d.y)).sqrt();
                let k = (len / f.outer) * 0.5;
                let m = fnms(k, k, 1.0);
                f.least = if m > 0.0 { m.sqrt() } else { 0.0 };
                f.inner = fnms(f.outer, f.least, first.radius);
            }
        }
    }
}

/// The shape of a wheel of that size on the front or the rear axle, sized once and kept.
/// A shape carries the sizes it has had in its last digits: it is given the size every wheel
/// bound has had first (below), then this one. MCLA_DBG_WHEELSIZES="r,w;r,w/r,w" names other
/// sizes for the front / the rear wheels' shapes to have had before (the fitted histories of
/// work/veh/gjk_exact.md part 2, which give the same numbers).
pub fn wheel_shape(front: bool, radius: f32, width: f32) -> WheelShape {
    static KEPT: std::sync::Mutex<Vec<((bool, u32, u32), WheelShape)>> = std::sync::Mutex::new(Vec::new());
    static PAST: std::sync::OnceLock<[Vec<(f32, f32)>; 2]> = std::sync::OnceLock::new();
    let key = (front, radius.to_bits(), width.to_bits());
    let mut kept = KEPT.lock().unwrap();
    if let Some(found) = kept.iter().find(|k| k.0 == key) {
        return found.1;
    }
    let past = PAST.get_or_init(|| {
        // (what every wheel's bound has been before its fitted size: the size the wheel tuning
        // is made with, TireRadius 0.3 and TireWidth 0.1 (0x82565348) - the car is built and
        // its bounds sized, 0x8232D068, before its set-up is fitted, 0x8236BC20. The tuning
        // file's own TireRadius never reaches the bound. MCLA_DBG_OLDWHEELSIZES: no past.)
        let Ok(text) = std::env::var("MCLA_DBG_WHEELSIZES") else {
            if std::env::var_os("MCLA_DBG_OLDWHEELSIZES").is_some() {
                return [Vec::new(), Vec::new()];
            }
            let made = (f32::from_bits(0x3e99_999a), f32::from_bits(0x3dcc_cccd));
            return [vec![made], vec![made]];
        };
        let mut axles = text.split('/').map(|a| a.split(';').filter_map(|s| s.split_once(',').and_then(|(r, w)| Some((r.trim().parse().ok()?, w.trim().parse().ok()?)))).collect::<Vec<(f32, f32)>>());
        [axles.next().unwrap_or_default(), axles.next().unwrap_or_default()]
    });
    let mut shape = WheelShape::sized(&past[if front { 0 } else { 1 }]);
    shape.resize(radius, width);
    kept.push((key, shape));
    shape
}

/// (0x820022B4)
const MILLIMETRE: f32 = f32::from_bits(0x3a83126f);

/// A wheel's core (the curved bound, type 5, scaled to the tyre by 0x825D30F8): a tread that
/// is a torus - round about the axle and an arc across - two rims, and sides that are caps of
/// spheres. Exactly round: the support point (0x825D1EE8) is that of the true surface.
#[derive(Clone, Copy, Debug)]
pub struct WheelCore {
    pub centre: Vec3,
    /// the wheel's axle, up and way
    pub axes: [Vec3; 3],
    radius: f32,
    width: f32,
    corners: [Vec3; 4],
    edges: [Edge; 6],
    faces: [Face; 4],
}

/// A wheel shape's margin.
pub const WHEEL_MARGIN: f32 = 0.04;

impl WheelCore {
    /// `radius`, `width`: the tuning's TireRadius and TireWidth as fitted; the unit shape
    /// resized to them at once.
    pub fn new(centre: Vec3, axes: [Vec3; 3], radius: f32, width: f32) -> Self {
        Self::of(&WheelShape::sized(&[(radius, width)]), centre, axes, radius, width)
    }

    /// A shape already sized, stood at `centre` along `axes`.
    pub fn of(shape: &WheelShape, centre: Vec3, axes: [Vec3; 3], radius: f32, width: f32) -> Self {
        WheelCore { centre, axes, radius, width, corners: shape.corners, edges: shape.edges, faces: shape.faces }
    }

    /// The length of the shape's box's half sizes (half width, radius, radius): the ball the grid looks cells up with.
    pub fn ball(&self) -> f32 {
        Vec3::new(self.width * 0.5, self.radius, self.radius).length()
    }

    /// The core's radius in the tyre's middle.
    pub fn round(&self) -> f32 {
        self.faces[2].outer + self.faces[2].inner
    }

    /// (for checking the shape against the original's: half width, rim radius, the tread's
    /// radius, inner radius and least cosine, the caps' centre, radius and least cosine)
    #[cfg(test)]
    fn numbers(&self) -> [u32; 8] {
        [self.corners[0].x, self.edges[0].radius, self.faces[2].outer, self.faces[2].inner, self.faces[2].least, self.faces[0].centre.x, self.faces[0].outer, self.faces[0].least].map(f32::to_bits)
    }

    /// 0x825D1EE8: the core's point farthest along `dir`, in the wheel's own frame. Corners,
    /// then curved edges, then curved faces; a later candidate wins only by being farther.
    fn farthest(&self, dir: Vec3) -> Vec3 {
        let d = dir * rsqrt_or_zero(dot(dir, dir));
        let (mut best, mut out) = (-f32::MAX, Vec3::ZERO);
        for v in self.corners {
            let t = dot(d, v);
            if t > best {
                best = t;
                out = v;
            }
        }
        for e in &self.edges {
            if !(e.radius > 0.0) {
                continue;
            }
            // the direction laid into the arc's plane
            let p = cross(cross(e.normal, d), e.normal);
            let l2 = dot(p, p);
            if !(l2 > FLAT) {
                continue;
            }
            let q = (p * e.radius) * rsqrt_or_zero(l2);
            let point = q + e.centre;
            let (a, b) = (self.corners[e.ends[0]] - e.centre, self.corners[e.ends[1]] - e.centre);
            let (s1, s2, s3) = (dot(e.normal, cross(a, q)), dot(e.normal, cross(a, b)), dot(e.normal, cross(q, b)));
            let (n1, n2, n3) = (0.0 > s1, 0.0 > s2, 0.0 > s3);
            let on = (!n2 && !n1 && !n3) || (!(n1 && n3) && n2) || e.ends[0] == e.ends[1];
            if dot(d, point) > best && on {
                best = dot(d, point);
                out = point;
            }
        }
        for f in &self.faces {
            if !(f.outer > 0.0) {
                continue;
            }
            if f.cap {
                if dot(f.normal, d) > f.least {
                    let point = d * f.outer + f.centre;
                    let t = dot(d, point);
                    if t > best {
                        best = t;
                        out = point;
                    }
                }
            } else {
                let n = self.edges[f.edges[0]].normal;
                let p = cross(cross(n, d), n);
                let l2 = dot(p, p);
                let rs = rsqrt_or_zero(l2);
                let u = if l2 > f32::from_bits(0x3727c5ac) { p * rs } else { Vec3::ZERO };
                let point = d * f.outer + (u * f.inner + f.centre);
                let t = dot(d, point);
                if t > best && dot(d, u) > f.least {
                    best = t;
                    out = point;
                }
            }
        }
        out
    }
}

impl Support for WheelCore {
    fn support(&self, dir: Vec3) -> Vec3 {
        self.support_at(dir, Vec3::ZERO, Vec3::ZERO)
    }

    /// The wheel's matrix applied the original's way: its place is (centre + `by`) + `then`,
    /// and each coordinate one sum of four products.
    fn support_at(&self, dir: Vec3, by: Vec3, then: Vec3) -> Vec3 {
        let at = (self.centre + by) + then;
        let [x, y, z] = self.axes;
        let p = self.farthest(Vec3::new(dot(x, dir), dot(y, dir), dot(z, dir)));
        Vec3::new((at.x + z.x * p.z) + (y.x * p.y + x.x * p.x), (at.y + z.y * p.z) + (y.y * p.y + x.y * p.x), (at.z + z.z * p.z) + (y.z * p.y + x.z * p.x))
    }

    fn describe(&self) -> Option<[u32; 14]> {
        let [x, y, z] = self.axes;
        Some([self.radius, self.width, x.x, x.y, x.z, y.x, y.y, y.z, z.x, z.y, z.z, self.centre.x, self.centre.y, self.centre.z].map(f32::to_bits))
    }
}

/// A wheel's matrix among the car's parts (0x8255CEA0, the last thing a vehicle update does):
/// the wheel's world rows and place `wheel` taken back through the chassis's matrix `car` of
/// that update - rows by its transpose, the place by that and minus the car's own place so
/// taken. This is what the car's bound keeps for the wheel's shape until the next update.
pub fn part_matrix(wheel: &[Vec3; 4], car: &[Vec3; 4]) -> [Vec3; 4] {
    let [c0, c1, c2, at] = *car;
    let (cx, cy, cz) = (Vec3::new(c0.x, c1.x, c2.x), Vec3::new(c0.y, c1.y, c2.y), Vec3::new(c0.z, c1.z, c2.z));
    let back = Vec3::ZERO - Vec3::new(dot(c0, at), dot(c1, at), dot(c2, at));
    let row = |w: Vec3| cz * w.z + (cy * w.y + cx * w.x);
    [row(wheel[0]), row(wheel[1]), row(wheel[2]), cz * wheel[3].z + (cy * wheel[3].y + (cx * wheel[3].x + back))]
}

/// A part's matrix in the world as the collision forms it (0x82575450, from 0x825CCE00): the
/// part's matrix among the car's parts times the body's matrix `car` as it stands after the
/// physics step.
pub fn world_matrix(part: &[Vec3; 4], car: &[Vec3; 4]) -> [Vec3; 4] {
    let row = |p: Vec3| car[2] * p.z + (car[1] * p.y + car[0] * p.x);
    [row(part[0]), row(part[1]), row(part[2]), car[2] * part[3].z + (car[1] * part[3].y + (car[0] * part[3].x + car[3]))]
}

/// A mesh triangle's normal as the collision forms it (0x825D0C18, 0x82576FF8): the cross
/// product of two of its sides over twice the area its record holds - a unit vector only as
/// nearly as that area is the triangle's.
pub fn polygon_normal(tri: &[Vec3; 3], area: f32) -> Vec3 {
    cross(tri[2] - tri[1], tri[0] - tri[1]) * recip(area + area)
}

/// 0x82576FF8: the direction between a triangle's normal `face` and its neighbour's `other`
/// across the edge `from` -> `to`, where the two meet in a ridge (and that they do); the
/// triangle's own normal where they meet flat or in a hollow.
pub fn edge_bisector(face: Vec3, other: Vec3, from: Vec3, to: Vec3) -> (Vec3, bool) {
    if dot(cross(other, to - from), face) > 0.0 {
        let s = face + other;
        (s * rsqrt(dot(s, s)), true)
    } else {
        (face, false)
    }
}

/// A triangle's corner farthest along `dir` (0x825E061C: of equals, the later one).
fn corner(tri: &[Vec3; 3], dir: Vec3) -> Vec3 {
    let t = [dot(dir, tri[0]), dot(dir, tri[1]), dot(dir, tri[2])];
    let (i, rest) = if t[0] - t[1] > 0.0 { (0, t[0] - t[2]) } else { (1, t[1] - t[2]) };
    if rest > 0.0 { tri[i] } else { tri[2] }
}

/// What the loop gives: the points on the two surfaces (margins applied), the unit vector from
/// the triangle to the shape, and the distance between the surfaces (negative: how deep).
#[derive(Clone, Copy, Debug)]
pub struct Nearest {
    pub on_a: Vec3,
    pub on_b: Vec3,
    pub normal: Vec3,
    pub distance: f32,
    /// the loop ended on one of its "this cannot be trusted" tests
    pub degenerate: bool,
    /// the answer is the penetration routine's
    pub deep: bool,
    /// the first shape's corners (their numbers) the simplex ended with, and how many
    pub corners: ([u32; 4], usize),
    /// the second shape's corners likewise
    pub corners_b: ([u32; 4], usize),
}

/// The loop alone: `shape` (moved by `by`, then `then`) against `tri`, both in a frame whose
/// origin is the triangle's first corner; `first` the first direction tried; the two margins.
/// A polygon as the loop's second shape (bound type 13, 0x825E061C).
pub struct Corners<'a>(pub &'a [Vec3; 3]);

impl Support for Corners<'_> {
    fn support(&self, dir: Vec3) -> Vec3 {
        corner(self.0, dir)
    }
    fn support_id(&self, dir: Vec3, _by: Vec3, _then: Vec3) -> (Vec3, u32) {
        (corner(self.0, dir), 0)
    }
}

fn loop_only(shape: &impl Support, by: Vec3, then: Vec3, tri: &[Vec3; 3], first: Vec3, margin_a: f32, margin_b: f32) -> (Option<Nearest>, bool) {
    loop_pair(shape, by, then, &Corners(tri), first, margin_a, margin_b)
}

/// The loop for any two convex cores (0x825E0150): the first's point farthest against `v`, the
/// second's farthest along it.
pub fn loop_pair(shape: &impl Support, by: Vec3, then: Vec3, other: &impl Support, first: Vec3, margin_a: f32, margin_b: f32) -> (Option<Nearest>, bool) {
    let mut v = first;
    loop_from(shape, by, then, other, &mut v, margin_a, margin_b)
}

/// `loop_pair` with the direction it starts from handed in and the one it ends with handed back
/// (the detector object's +16, which the swept test keeps from one call to the next).
pub fn loop_from(shape: &impl Support, by: Vec3, then: Vec3, other: &impl Support, dir: &mut Vec3, margin_a: f32, margin_b: f32) -> (Option<Nearest>, bool) {
    let (found, degenerate, _) = loop_ends(shape, by, then, other, dir, margin_a, margin_b);
    (found, degenerate)
}

/// The corner numbers of the two shapes the simplex holds (simplex +372 / +392 and its count +412).
pub type Ends = (([u32; 4], usize), ([u32; 4], usize));

/// `loop_from` that also gives the corners the simplex ended with, whether or not there is an answer: the
/// simplex buffer is the caller's, and a penetration routine's inner loop leaves its own corners in it.
fn loop_ends(shape: &impl Support, by: Vec3, then: Vec3, other: &impl Support, dir: &mut Vec3, margin_a: f32, margin_b: f32) -> (Option<Nearest>, bool, Ends) {
    let mut simplex = Simplex::default();
    let mut v = *dir;
    let mut squared = f32::MAX;
    let mut degenerate = false;
    let mut check = false;
    let mut iterations = 0;
    loop {
        let (p, id) = shape.support_id(Vec3::ZERO - v, by, then);
        let (q, id_b) = other.support_id(v, Vec3::ZERO, Vec3::ZERO);
        let w = p - q;
        let delta = dot(v, w);
        // (the original's "farther than the greatest distance asked for" exit cannot be
        // taken: the distance asked for is the greatest single number)
        if simplex.has(w) {
            degenerate = true;
            check = true;
            break;
        }
        let f0 = squared - delta;
        if squared * REL >= f0 {
            if 0.0 >= f0 {
                degenerate = true;
            }
            check = true;
            break;
        }
        simplex.add(w, p, q);
        simplex.ids[simplex.count - 1] = id;
        simplex.ids_b[simplex.count - 1] = id_b;
        if !simplex.closest() {
            degenerate = true;
            check = true;
            break;
        }
        let before = squared;
        v = simplex.v;
        *dir = v;
        squared = dot(v, v);
        if f32::EPSILON * before >= before - squared {
            check = true;
            break;
        }
        iterations += 1;
        if iterations > 1001 {
            break;
        }
        if simplex.count == 4 {
            break;
        }
        let widest = (0..simplex.count).map(|i| dot(simplex.w[i], simplex.w[i])).fold(0.0, f32::max);
        if f32::EPSILON * widest >= squared {
            break;
        }
    }
    let ends = ((simplex.ids, simplex.count), (simplex.ids_b, simplex.count));
    if !check {
        return (None, degenerate, ends);
    }
    let len2 = dot(v, v);
    if NEAR2 > len2 {
        degenerate = true;
    }
    if !(len2 > f32::EPSILON) {
        return (None, degenerate, ends);
    }
    let rs = rsqrt(len2);
    let len = recip(rs);
    let (a, b) = (simplex.p1, simplex.p2);
    let on_a = -(Vec3::splat(margin_a * rs) * v - a);
    let on_b = v * Vec3::splat(margin_b * rs) + b;
    (Some(Nearest { on_a, on_b, normal: (a - b) * rs, distance: (len - margin_a) - margin_b, degenerate, deep: false, corners: ends.0, corners_b: ends.1 }), degenerate, ends)
}

/// (MCLA_DBG_GJK=<file>: every wheel's call of `contact` is written there as one line of
/// tools/gjk_reference.py's cases)
fn record(shape: &impl Support, tri: &[Vec3; 3], face: Vec3, bisectors: &[Vec3; 3]) {
    use std::io::Write;
    static OUT: std::sync::OnceLock<Option<std::sync::Mutex<std::fs::File>>> = std::sync::OnceLock::new();
    let Some(out) = OUT.get_or_init(|| std::env::var_os("MCLA_DBG_GJK").and_then(|p| std::fs::OpenOptions::new().create(true).append(true).open(p).ok()).map(std::sync::Mutex::new)) else { return };
    let Some(wheel) = shape.describe() else { return };
    let mut line = String::new();
    let vectors = tri.iter().chain([&face]).chain(bisectors.iter());
    for bits in wheel.into_iter().chain(vectors.flat_map(|v| v.to_array().map(f32::to_bits))) {
        line.push_str(&format!("{bits:08x} "));
    }
    let _ = writeln!(out.lock().unwrap(), "{}", line.trim_end());
}

/// One polygon against one shape (0x825CEE38's use of the detector): the loop, and where the
/// cores overlap - or the loop ended untrustworthy with the cores under a centimetre apart -
/// the penetration routine (0x825E2370), whose answer is taken if it is the deeper.
/// `face`: the polygon's normal; `bisectors`: for each of its edges the direction between its
/// normal and the neighbour's across that edge where they meet in a ridge, else its normal.
/// The points are given about the polygon's first corner, as the routine itself has them.
pub fn contact_about(shape: &impl Support, tri: &[Vec3; 3], face: Vec3, bisectors: [Vec3; 3], margin_a: f32, margin_b: f32) -> Option<Nearest> {
    record(shape, tri, face, &bisectors);
    // (the original's frame: the world's axes, the polygon's first corner the origin)
    let origin = tri[0];
    let local = [Vec3::ZERO, tri[1] - origin, tri[2] - origin];
    let by = Vec3::ZERO - origin;
    let (mut result, degenerate) = loop_only(shape, by, Vec3::ZERO, &local, Vec3::X, margin_a, margin_b);
    let catch = degenerate && result.map_or(false, |r| CATCH > (r.distance + margin_a) + margin_b);
    if result.is_none() || catch {
        // the ways out that are tried: the polygon's normal, its edges' bisectors as given
        // and the three between each two of those (unit vectors, by the estimate)
        let between = |a: Vec3, b: Vec3| {
            let s = a + b;
            s * rsqrt(dot(s, s))
        };
        let dirs = [face, bisectors[0], bisectors[1], bisectors[2], between(bisectors[0], bisectors[1]), between(bisectors[0], bisectors[2]), between(bisectors[1], bisectors[2])];
        let mut best = (f32::MAX, face);
        for d in dirs {
            let p = shape.support_at(Vec3::ZERO - d, by, Vec3::ZERO);
            let delta = dot(d, corner(&local, d) - p);
            if best.0 > delta {
                best = (delta, d);
            }
        }
        let off = best.0 + ((margin_a + margin_b) + CATCH);
        let mut first = Vec3::Z;
        let (inner, _, ends) = loop_ends(shape, by, best.1 * off, &Corners(&local), &mut first, margin_a, margin_b);
        // (the routine's inner detector works in the caller's simplex buffer: the element of whichever answer is
        // kept is read from the corners that loop ended with, 0x825E1E10 after 0x825E2370)
        if let Some(r) = result.as_mut() {
            (r.corners, r.corners_b) = ends;
        }
        if let Some(r) = inner {
            let pen = off * dot(best.1, r.normal) - r.distance;
            let b = r.on_b + Vec3::ZERO;
            let a = -(Vec3::splat(pen) * r.normal - b);
            let between = b - a;
            let l2 = dot(between, between);
            if l2 > LEAST2 {
                let l = l2 * rsqrt(l2);
                if result.map_or(true, |g| g.distance > 0.0 - l) {
                    result = Some(Nearest { on_a: a, on_b: b, normal: between * recip(l), distance: 0.0 - l, degenerate, deep: true, corners: r.corners, corners_b: r.corners_b });
                }
            }
        }
    }
    result
}

/// `contact_about` with the points given in the world (the polygon's first corner added back,
/// as 0x825CEE38 hands them to the manifold).
pub fn contact(shape: &impl Support, tri: &[Vec3; 3], face: Vec3, bisectors: [Vec3; 3], margin_a: f32, margin_b: f32) -> Option<Nearest> {
    contact_about(shape, tri, face, bisectors, margin_a, margin_b).map(|mut r| {
        r.on_a += tri[0];
        r.on_b += tri[0];
        r
    })
}

/// The simplex of the loop (Bullet's Voronoi simplex solver with the original's two numbers):
/// up to four points of the difference of the two shapes, each with the points of the shapes
/// it came from; and what of it is nearest the origin.
#[derive(Clone, Copy, Debug)]
struct Simplex {
    count: usize,
    w: [Vec3; 4],
    p: [Vec3; 4],
    q: [Vec3; 4],
    last: Vec3,
    /// the numbers of the first shape's corners the points came from
    ids: [u32; 4],
    /// and the second shape's
    ids_b: [u32; 4],
    /// the nearest point (from the second shape to the first) and where it lies on each
    v: Vec3,
    p1: Vec3,
    p2: Vec3,
}

impl Default for Simplex {
    fn default() -> Self {
        Simplex { count: 0, w: [Vec3::ZERO; 4], p: [Vec3::ZERO; 4], q: [Vec3::ZERO; 4], last: Vec3::splat(f32::MAX), ids: [0; 4], ids_b: [0; 4], v: Vec3::ZERO, p1: Vec3::ZERO, p2: Vec3::ZERO }
    }
}

impl Simplex {
    /// 0x825E1660: is `w` one of the simplex's points or the last one tried - to within a
    /// 1024th of the simplex's reach, each component.
    fn has(&self, w: Vec3) -> bool {
        let reach = (0..self.count).map(|i| dot(self.w[i], self.w[i])).fold(1.0, f32::max);
        let tol = Vec3::splat((1.0 / 1024.0) * (reach * rsqrt(reach)));
        let near = |x: Vec3| (x + tol).cmpge(w).all() && w.cmpge(x - tol).all();
        (0..self.count).fold(false, |any, i| any | near(self.w[i])) | near(self.last)
    }

    fn add(&mut self, w: Vec3, p: Vec3, q: Vec3) {
        self.last = w;
        self.w[self.count] = w;
        self.p[self.count] = p;
        self.q[self.count] = q;
        self.count += 1;
    }

    /// (the unused go, the last first; the simplex's last point moves into each hole)
    fn keep(&mut self, used: [bool; 4]) {
        for i in (0..self.count).rev() {
            if !used[i] {
                let last = self.count - 1;
                self.w[i] = self.w[last];
                self.p[i] = self.p[last];
                self.q[i] = self.q[last];
                self.ids[i] = self.ids[last];
                self.ids_b[i] = self.ids_b[last];
                self.count -= 1;
            }
        }
    }

    /// 0x825E1F00: the simplex's point nearest the origin; the corners that do not carry it
    /// are dropped. False where four points lie all but flat (the loop stops there with what
    /// it had).
    fn closest(&mut self) -> bool {
        let (w, p, q) = (self.w, self.p, self.q);
        let (bary, used): ([f32; 4], [bool; 4]) = match self.count {
            1 => {
                self.p1 = p[0];
                self.p2 = q[0];
                self.v = p[0] - q[0];
                return true;
            }
            2 => {
                let ab = w[1] - w[0];
                let mut t = dot(ab, Vec3::ZERO - w[0]);
                let used;
                if t > 0.0 {
                    let dd = dot(ab, ab);
                    if dd > t {
                        t *= recip(dd);
                        used = [true, true, false, false];
                    } else {
                        t = 1.0;
                        used = [false, true, false, false];
                    }
                } else {
                    t = 0.0;
                    used = [true, false, false, false];
                }
                let t = Vec3::splat(t);
                self.p1 = t * (p[1] - p[0]) + p[0];
                self.p2 = t * (q[1] - q[0]) + q[0];
                self.v = self.p1 - self.p2;
                self.keep(used);
                return true;
            }
            3 => {
                let (_, b) = on_triangle(w[0], w[1], w[2]);
                ([b[0], b[1], b[2], 0.0], [b[0] > 0.0, b[1] > 0.0, b[2] > 0.0, false])
            }
            4 => {
                // which faces the origin lies outside of (0x825E19B8)
                let outside = |a: Vec3, b: Vec3, c: Vec3, d: Vec3| -> Option<bool> {
                    let n = cross(b - a, c - a);
                    let sd = dot(d - a, n);
                    if FLAT > dot(Vec3::splat(sd), Vec3::splat(sd)) {
                        return None;
                    }
                    Some(0.0 > dot(Vec3::ZERO - a, n) * sd)
                };
                let faces: [[usize; 4]; 4] = [[0, 1, 2, 3], [0, 2, 3, 1], [0, 3, 1, 2], [1, 3, 2, 0]];
                let mut sides = [false; 4];
                for (side, f) in sides.iter_mut().zip(faces) {
                    match outside(w[f[0]], w[f[1]], w[f[2]], w[f[3]]) {
                        Some(s) => *side = s,
                        None => return false,
                    }
                }
                if sides.iter().all(|s| !*s) {
                    // the origin is inside: the cores overlap
                    self.v = Vec3::ZERO;
                    return true;
                }
                let mut best = (f32::MAX, [0.0; 4]);
                for (face, side) in faces.iter().zip(sides) {
                    if !side {
                        continue;
                    }
                    let (at, b) = on_triangle(w[face[0]], w[face[1]], w[face[2]]);
                    let d2 = dot(at, at);
                    if best.0 > d2 {
                        let mut bary = [0.0; 4];
                        for k in 0..3 {
                            bary[face[k]] = b[k];
                        }
                        best = (d2, bary);
                    }
                }
                (best.1, best.1.map(|b| b > 0.0))
            }
            _ => return false,
        };
        let mut a = p[1] * bary[1] + p[0] * bary[0];
        let mut b = q[1] * bary[1] + q[0] * bary[0];
        for i in 2..self.count {
            a = p[i] * bary[i] + a;
            b = q[i] * bary[i] + b;
        }
        self.p1 = a;
        self.p2 = b;
        self.v = a - b;
        self.keep(used);
        bary.iter().all(|b| *b >= 0.0)
    }
}

/// 0x825E1768: a triangle's point nearest the origin and its weights on the corners
/// (Ericson's regions, as the simplex solver has them; divisions by the estimate of 1 / x).
fn on_triangle(a: Vec3, b: Vec3, c: Vec3) -> (Vec3, [f32; 3]) {
    let (ab, ac) = (b - a, c - a);
    let (ap, bp, cp) = (Vec3::ZERO - a, Vec3::ZERO - b, Vec3::ZERO - c);
    let (d1, d2) = (dot(ab, ap), dot(ac, ap));
    let (d3, d4) = (dot(ab, bp), dot(ac, bp));
    let (d5, d6) = (dot(ab, cp), dot(ac, cp));
    if !(d1 > 0.0 || d2 > 0.0) {
        return (a, [1.0, 0.0, 0.0]);
    }
    if !(0.0 > d3 || d4 > d3) {
        return (b, [0.0, 1.0, 0.0]);
    }
    let vc = d1 * d4 - d3 * d2;
    if !(vc > 0.0 || 0.0 > d1 || d3 > 0.0) {
        let v = d1 * recip(d1 - d3);
        return (ab * v + a, [1.0 - v, v, 0.0]);
    }
    if !(0.0 > d6 || d5 > d6) {
        return (c, [0.0, 0.0, 1.0]);
    }
    let vb = d5 * d2 - d1 * d6;
    if !(vb > 0.0 || 0.0 > d2 || d6 > 0.0) {
        let w = d2 * recip(d2 - d6);
        return (ac * w + a, [1.0 - w, 0.0, w]);
    }
    let va = d3 * d6 - d5 * d4;
    let (e1, e2) = (d4 - d3, d5 - d6);
    if !(va > 0.0 || 0.0 > e1 || 0.0 > e2) {
        let w = e1 * recip(e1 + e2);
        return ((c - b) * w + b, [0.0, 1.0 - w, w]);
    }
    let denom = 1.0 * recip((va + vb) + vc);
    let (v, w) = (vb * denom, vc * denom);
    (ac * w + (ab * v + a), [1.0 - (v + w), v, w])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wheel(centre: Vec3) -> WheelCore {
        WheelCore::new(centre, [Vec3::X, Vec3::Y, Vec3::Z], 0.3443, 0.245)
    }

    fn crown() -> f32 {
        // sqrt(w^2/4 + 6 s^2) - (sqrt 6 - 1) s + 0.04
        let s = 0.3443f32 - 0.04;
        (0.245f32 * 0.245 * 0.25 + 6.0 * s * s).sqrt() - (6.0f32.sqrt() - 1.0) * s + 0.04
    }

    #[test]
    fn a_wheel_on_flat_ground_stands_on_its_crown() {
        let tri = [Vec3::new(-3.0, 0.0, -3.0), Vec3::new(0.0, 0.0, 4.0), Vec3::new(3.0, 0.0, -3.0)];
        let w = wheel(Vec3::new(0.1, 0.36, 0.2));
        assert!((w.round() + 0.04 - crown()).abs() < 1e-5);
        let r = contact(&w, &tri, Vec3::Y, [Vec3::Y; 3], 0.04, 0.005).unwrap();
        assert!((r.distance - (0.36 - crown() - 0.005)).abs() < 2e-4, "{r:?}");
        assert!((r.normal - Vec3::Y).length() < 1e-2, "{r:?}");
    }

    #[test]
    fn against_an_edge_the_loop_stops_short_of_the_round() {
        // a plate that ends 11 cm behind the axle, as recorded on an S281's front wheel:
        // never nearer than the tyre's round is, and no more than 1.5 mm over that
        let tri = [Vec3::new(-2.0, 0.0, 0.11), Vec3::new(0.0, 0.0, 3.0), Vec3::new(2.0, 0.0, 0.11)];
        let w = wheel(Vec3::new(0.0, 0.345, 0.0));
        let r = contact(&w, &tri, Vec3::Y, [Vec3::Y; 3], 0.04, 0.005).unwrap();
        let exact = (0.345f32 * 0.345 + 0.11 * 0.11).sqrt() - crown() - 0.005;
        assert!(r.distance >= exact - 1e-5 && r.distance < exact + 0.0015, "{} against {exact}", r.distance);
    }

    #[test]
    fn a_wheel_deep_in_a_polygon_is_measured_backed_out() {
        let tri = [Vec3::new(-3.0, 0.0, -3.0), Vec3::new(0.0, 0.0, 4.0), Vec3::new(3.0, 0.0, -3.0)];
        let w = wheel(Vec3::new(0.0, 0.2, 0.0));
        let r = contact(&w, &tri, Vec3::Y, [Vec3::Y; 3], 0.04, 0.005).unwrap();
        assert!(r.deep);
        assert!((-r.distance - (crown() + 0.005 - 0.2)).abs() < 5e-4, "{r:?}");
    }

    /// Against the original routine run offline on the same inputs (tools/gjk_reference.py):
    /// every bit of the answer - whether there is one, its normal, both points, the distance,
    /// the two flags - and of the resized shape's numbers.
    #[test]
    #[ignore = "requires MCLA_GJK_REFERENCE from tools/gjk_reference.py"]
    fn the_narrow_phase_is_the_originals_bit_for_bit() {
        let path = std::env::var("MCLA_GJK_REFERENCE").expect("MCLA_GJK_REFERENCE");
        let text = std::fs::read_to_string(path).unwrap();
        let hex = |s: &str| u32::from_str_radix(s, 16).unwrap();
        let (mut rows, mut wrong, mut shapes) = (0, 0, 0);
        for line in text.lines().skip(1) {
            let c: Vec<&str> = line.split(',').collect();
            assert_eq!(c.len(), 24 + 35, "{line}");
            let flag = |i: usize| c[i] == "1";
            let out: Vec<u32> = c[6..24].iter().map(|s| hex(s)).collect();
            let f: Vec<f32> = c[24..].iter().map(|s| f32::from_bits(hex(s))).collect();
            let v = |i: usize| Vec3::new(f[i], f[i + 1], f[i + 2]);
            // (GJK_HISTORY="r,w;r,w", as for the tool: the sizes the shape had before)
            let mut sizes: Vec<(f32, f32)> = std::env::var("GJK_HISTORY").unwrap_or_default().split(';').filter_map(|s| s.split_once(',').and_then(|(r, w)| Some((r.trim().parse().ok()?, w.trim().parse().ok()?)))).collect();
            sizes.push((f[0], f[1]));
            let core = WheelCore::of(&WheelShape::sized(&sizes), v(11), [v(2), v(5), v(8)], f[0], f[1]);
            if core.numbers()[..] != out[10..18] {
                shapes += 1;
                if shapes <= 5 {
                    eprintln!("{}: shape {:08x?} against {:08x?}", c[0], core.numbers(), &out[10..18]);
                }
            }
            let r = contact(&core, &[v(14), v(17), v(20)], v(23), [v(26), v(29), v(32)], WHEEL_MARGIN, 0.005);
            let mine: Option<([u32; 10], bool, bool)> = r.map(|r| {
                let n = [r.normal, r.on_a, r.on_b].map(|p| p.to_array().map(f32::to_bits));
                ([n[0][0], n[0][1], n[0][2], n[1][0], n[1][1], n[1][2], n[2][0], n[2][1], n[2][2], r.distance.to_bits()], r.degenerate, r.deep)
            });
            let theirs = flag(1).then(|| (<[u32; 10]>::try_from(&out[..10]).unwrap(), flag(2), flag(3)));
            rows += 1;
            if mine != theirs {
                wrong += 1;
                if wrong <= 10 {
                    eprintln!("{}:\n  mine   {:08x?}\n  theirs {:08x?}", c[0], mine, theirs);
                }
            }
        }
        eprintln!("{rows} cases: {wrong} differ, {shapes} shapes differ");
        assert!(rows > 0 && wrong == 0 && shapes == 0);
    }
}

/// A sphere, capsule or box part of a prop's bound as the detector has it (0x825E0384, 0x825E038C,
/// 0x825E03D4): the core's point in the part's own frame, carried into the world by the part's
/// matrix. The margin (the sphere's or capsule's radius, the box's +0x70) is added by the loop.
#[derive(Clone, Copy, Debug)]
pub struct PartCore {
    /// bound +4
    pub kind: u8,
    pub centroid: Vec3,
    /// a box's half sizes less its margin; a capsule's length along y
    pub half: Vec3,
    pub rows: [Vec3; 3],
    pub at: Vec3,
}

impl PartCore {
    /// The core's point in the part's frame farthest along the local direction `d`.
    pub fn local(&self, d: Vec3) -> Vec3 {
        match self.kind {
            // (a capsule: the centroid and half its length up or down its second axis; down only
            // for a direction that points down)
            1 => self.half * Vec3::new(0.0, if 0.0 > d.y { -0.5 } else { 0.5 }, 0.0) + self.centroid,
            3 => {
                let (up, down) = (self.centroid + self.half, self.centroid - self.half);
                Vec3::new(if d.x > 0.0 { up.x } else { down.x }, if d.y > 0.0 { up.y } else { down.y }, if d.z > 0.0 { up.z } else { down.z })
            }
            _ => self.centroid,
        }
    }
}

impl Support for PartCore {
    fn support(&self, dir: Vec3) -> Vec3 {
        self.support_at(dir, Vec3::ZERO, Vec3::ZERO)
    }

    fn support_at(&self, dir: Vec3, by: Vec3, then: Vec3) -> Vec3 {
        let at = (self.at + by) + then;
        let [x, y, z] = self.rows;
        let p = self.local(Vec3::new(dot(x, dir), dot(y, dir), dot(z, dir)));
        Vec3::new((at.x + z.x * p.z) + (y.x * p.y + x.x * p.x), (at.y + z.y * p.z) + (y.y * p.y + x.y * p.x), (at.z + z.z * p.z) + (y.z * p.y + x.z * p.x))
    }
}

/// The 42 directions the convex-against-convex penetration routine tries (0x82812D70).
const WAYS_OUT: [[u32; 3]; 42] = [
    [0x00000000, 0x80000000, 0xbf800000],
    [0x3f393e60, 0xbf0695ea, 0xbee4f9e4],
    [0xbe8d82ba, 0xbf59c422, 0xbee4f9e4],
    [0xbf64f91a, 0x80000000, 0xbee4f97f],
    [0xbe8d82ba, 0x3f59c422, 0xbee4fa05],
    [0x3f393e60, 0x3f0695ea, 0xbee4f9e4],
    [0x3e8d82ba, 0xbf59c422, 0x3ee4fa05],
    [0xbf393e60, 0xbf0695ea, 0x3ee4f9e4],
    [0xbf393e60, 0x3f0695ea, 0x3ee4f9e4],
    [0x3e8d82ba, 0x3f59c422, 0x3ee4f9e4],
    [0x3f64f91a, 0x00000000, 0x3ee4f97f],
    [0x80000000, 0x00000000, 0x3f800000],
    [0x3ed9c3f0, 0xbe9e36b1, 0xbf59c476],
    [0xbe265ade, 0xbeffff58, 0xbf59c476],
    [0x3e8696c4, 0xbf4f1b69, 0xbf0696c4],
    [0x3ed9c3f0, 0x3e9e36b1, 0xbf59c476],
    [0x3f59c411, 0x80000000, 0xbf0696a2],
    [0xbf06963e, 0x80000000, 0xbf59c454],
    [0xbf302d38, 0xbeffff9b, 0xbf0696a2],
    [0xbe265ade, 0x3effff58, 0xbf59c476],
    [0xbf302d38, 0x3effff9b, 0xbf0696a2],
    [0x3e8696c4, 0x3f4f1b69, 0xbf0696c4],
    [0x3f737889, 0x3e9e36f4, 0x00000000],
    [0x3f737889, 0xbe9e36f4, 0x00000000],
    [0x3f167925, 0xbf4f1bbd, 0x00000000],
    [0x00000000, 0xbf800000, 0x00000000],
    [0xbf167925, 0xbf4f1bbd, 0x00000000],
    [0xbf737889, 0xbe9e36f4, 0x80000000],
    [0xbf737889, 0x3e9e36f4, 0x80000000],
    [0xbf167925, 0x3f4f1bbd, 0x80000000],
    [0x80000000, 0x3f800000, 0x80000000],
    [0x3f167925, 0x3f4f1bbd, 0x80000000],
    [0x3f302d38, 0xbeffff9b, 0x3f0696a2],
    [0xbe8696c4, 0xbf4f1b69, 0x3f0696c4],
    [0xbf59c411, 0x00000000, 0x3f0696a2],
    [0xbe8696c4, 0x3f4f1b69, 0x3f0696c4],
    [0x3f302d38, 0x3effff9b, 0x3f0696a2],
    [0x3f06963e, 0x00000000, 0x3f59c454],
    [0x3e265ade, 0xbeffff58, 0x3f59c476],
    [0xbed9c3f0, 0xbe9e36b1, 0x3f59c476],
    [0xbed9c3f0, 0x3e9e36b1, 0x3f59c476],
    [0x3e265ade, 0x3effff58, 0x3f59c476],
];

/// Two convex parts against each other (0x825E4970's use of the detector 0x825E0150, in the
/// world's own coordinates): the loop from the direction `dir` (left at the one it ended with),
/// and where the cores overlap - or the loop ended untrustworthy with the cores under a
/// centimetre apart - the routine 0x825C67E8: of 42 fixed directions the one along which the
/// first shape is soonest out, the loop again with it moved out that far, and the answer brought
/// back; taken if it is the deeper. `routine`: whether the detector has that routine at all.
/// The points are on the two surfaces (margins applied), the normal is the way the first is pushed.
pub fn convex_contact(a: &impl Support, margin_a: f32, b: &impl Support, margin_b: f32, dir: &mut Vec3, routine: bool) -> Option<Nearest> {
    let (mut result, degenerate) = loop_from(a, Vec3::ZERO, Vec3::ZERO, b, dir, margin_a, margin_b);
    let catch = degenerate && result.map_or(false, |r| CATCH > (r.distance + margin_a) + margin_b);
    if (result.is_none() || catch) && routine {
        let mut best = (f32::MAX, Vec3::ZERO);
        for w in WAYS_OUT {
            let d = Vec3::new(f32::from_bits(w[0]), f32::from_bits(w[1]), f32::from_bits(w[2]));
            let pa = a.support(Vec3::ZERO - d);
            let pb = b.support(d);
            let delta = dot(d, pb - pa);
            if best.0 > delta {
                best = (delta, d);
            }
        }
        let mut off = best.0 + ((margin_a + margin_b) + CATCH);
        if 0.0 > off {
            off = 0.0;
        }
        let mut v = Vec3::Z;
        let (inner, _, ends) = loop_ends(a, best.1 * off, Vec3::ZERO, b, &mut v, margin_a, margin_b);
        // (the inner detector works in the caller's simplex buffer: whichever answer is kept, its element is read
        // from the corners this loop ended with, 0x825E1E10 after 0x825C67E8)
        if let Some(r) = result.as_mut() {
            (r.corners, r.corners_b) = ends;
        }
        if let Some(r) = inner {
            let pen = off * dot(best.1, r.normal) - r.distance;
            let (oa, ob) = (r.on_b - r.normal * pen, r.on_b);
            let between = ob - oa;
            let l2 = dot(between, between);
            if l2 > LEAST2 {
                let l = if l2 == 0.0 { 0.0 } else { l2 * rsqrt(l2) };
                if result.map_or(true, |g| g.distance > 0.0 - l) {
                    result = Some(Nearest { on_a: oa, on_b: ob, normal: between * recip(l), distance: 0.0 - l, degenerate, deep: true, corners: r.corners, corners_b: r.corners_b });
                }
            }
        }
    }
    result
}
