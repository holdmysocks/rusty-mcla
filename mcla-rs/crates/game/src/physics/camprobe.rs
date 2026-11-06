//! The level as the cameras' probes see it (rules in `docs/reference/cameras/probe_rule.md`): the
//! city and the other cars, a ball's contacts at a point (F-87651 -> F-82650, up to `max` kept by
//! the result list's score, F-89028), a ball swept along a segment (F-43031, directed) and the
//! one-sided line probe (F-20855). The camera's own car is left out by its identity.

use glam::{Mat3, Quat, Vec3};

use super::collision::WorldCollider;
use super::exact::{dot3 as vdot, dot_xyz, fma, fms, fnms, recip};
use super::hull::HullShape;

/// One car's hull in the world: its triangles in the file's order, their normals and the
/// neighbour across each edge, and the ball round it (the bound's sphere cull).
pub struct CamObj {
    tris: Vec<[Vec3; 3]>,
    normals: Vec<Vec3>,
    beside: Vec<[Option<u32>; 3]>,
    centre: Vec3,
    radius: f32,
}

impl CamObj {
    /// The hull at the car's place. (The neighbours are the triangles sharing each edge - the
    /// bound's polygon records name the same ones for a closed hull.)
    pub fn of_hull(hull: &HullShape, pos: Vec3, rot: Quat) -> CamObj {
        let m = Mat3::from_quat(rot);
        let w: Vec<Vec3> = hull.verts.iter().map(|&v| pos + m * v).collect();
        let tris: Vec<[Vec3; 3]> = hull.tris.iter().map(|t| [w[t[0] as usize], w[t[1] as usize], w[t[2] as usize]]).collect();
        let normals = tris.iter().map(|t| (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero()).collect();
        let beside = hull
            .tris
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let mut b = [None; 3];
                for e in 0..3 {
                    let (a, c) = (t[e], t[(e + 1) % 3]);
                    b[e] = hull.tris.iter().enumerate().position(|(j, u)| j != i && (0..3).any(|k| u[k] == c && u[(k + 1) % 3] == a)).map(|j| j as u32);
                }
                b
            })
            .collect();
        let centre = pos + m * hull.centre;
        let radius = w.iter().map(|v| v.distance(centre)).fold(0.0, f32::max);
        CamObj { tris, normals, beside, centre, radius }
    }

    /// A ball swept from `a` to `b` against this car alone, directed: the first touch, (t, normal).
    pub fn sweep(&self, a: Vec3, b: Vec3, r: f32) -> Option<(f32, Vec3)> {
        let d = b - a;
        if d.length_squared() < 1e-12 {
            return None;
        }
        // (the capsule a..b with r against the car's ball)
        let t = ((self.centre - a).dot(d) / d.length_squared()).clamp(0.0, 1.0);
        if (a + d * t).distance(self.centre) > self.radius + r {
            return None;
        }
        let mut best: Option<(f32, Vec3)> = None;
        for (i, v) in self.tris.iter().enumerate() {
            if let Some(h) = sweep_polygon(a, d, r, v, self.normals[i]) {
                if best.map_or(true, |b| h.0 < b.0) {
                    best = Some(h);
                }
            }
        }
        best
    }
}

/// A contact of a ball: where it touches, the contact normal and how deep.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contact {
    pub pos: Vec3,
    pub normal: Vec3,
    pub depth: f32,
}

/// The city plus the cars (other than the camera's).
pub struct CamLevel<'a> {
    pub city: &'a WorldCollider,
    pub cars: Vec<CamObj>,
}

/// The result list (F-89028): up to `max` records, the lowest score dropped; the score is the
/// depth, or cap - depth past the cap. Records stay in their slots (discovery order).
struct List {
    max: usize,
    cap: f32,
    held: Vec<(f32, Contact)>,
}

impl List {
    fn add(&mut self, c: Contact) {
        if !(c.depth > 0.0) || self.max == 0 {
            return;
        }
        let score = if c.depth > self.cap { self.cap - c.depth } else { c.depth };
        if self.held.len() < self.max {
            self.held.push((score, c));
            return;
        }
        let (k, low) = self.held.iter().enumerate().fold((0, f32::INFINITY), |(k, l), (i, h)| if h.0 < l { (i, h.0) } else { (k, l) });
        if score > low {
            self.held[k] = (score, c);
        }
    }
}

/// A ball against one polygon (F-82650), operation for operation: both sides of the plane; the
/// face, else an edge (taken only in front of the plane and only if the neighbour does not own
/// it, F-38934), else a corner. `n` the polygon's normal as the bound forms it (from the area its
/// record holds, `gjk::polygon_normal`); `own` the bound's normal of the query's polygon index (the
/// same polygon: `n`; past the bound's count (0, 0, 1)); `beside(e)` the normal of the polygon its
/// record names across edge e (None: none; past the count (0, 0, 1)). `flags` is the stack slot
/// of the edge flags (1 inside the edge, -1 before its start, -2 past its end, 0 else): an edge its
/// owner rejects leaves the later ones as the previous call left them, and the corner search reads
/// them. Checked to the bit against the original run offline (`ball_polygon_matches_the_original`).
pub fn ball_polygon(c: Vec3, r: f32, v: &[Vec3; 3], n: Vec3, own: Vec3, beside: impl Fn(usize) -> Option<Vec3>, flags: &mut [i32; 3]) -> Option<Contact> {
    let w0 = c - v[0];
    let d = fma(n.z, w0.z, fma(n.x, w0.x, n.y * w0.y));
    if d.abs() > r {
        return None;
    }
    let mut inside = true;
    let mut corner = false;
    for i in 0..3 {
        let (a, b) = (v[i], v[(i + 1) % 3]);
        let w = c - a;
        flags[i] = 0;
        let e = b - a;
        let m = vcross(n, e);
        if !(fma(m.z, w.z, fma(m.x, w.x, m.y * w.y)) < 0.0) {
            flags[i] = 1;
            continue;
        }
        inside = false;
        if fma(w.z, e.z, fma(e.x, w.x, e.y * w.y)) < 0.0 {
            flags[i] = -1;
            continue;
        }
        let u = c - b;
        if fma(u.z, e.z, fma(u.x, e.x, u.y * e.y)) > 0.0 {
            flags[i] = -2;
            continue;
        }
        if !(fma(n.z, w.z, fma(n.x, w.x, n.y * w.y)) > 0.0) {
            continue;
        }
        // the edge: the distance to its line, the point on it
        let ee = fma(e.z, e.z, fma(e.x, e.x, e.y * e.y));
        let ww = fma(w.z, w.z, fma(w.x, w.x, w.y * w.y));
        let uu = fma(u.z, u.z, fma(u.x, u.x, u.y * u.y));
        let inv = if ee == 0.0 { 1e8 } else { 1.0 / ee };
        let dw = ww - uu;
        let d2 = fnms(dw * dw, inv, fms(uu + ww, 2.0, ee)) * 0.25;
        if d2 > r * r {
            return None;
        }
        let dist = if d2 > 0.0 { d2.sqrt() } else { 0.0 };
        let depth = r - dist;
        let s2 = (ww - d2) * inv;
        let s = if s2 > 0.0 { s2.sqrt() } else { 0.0 };
        let q = e * s + a;
        let normal = if dist > 1e-12 { (c - q) * recip(dist) } else { n };
        if !(edge_owner(normal, depth, beside(i), own) < 0.0) {
            return Some(Contact { pos: q, normal, depth });
        }
        corner = true;
        break;
    }
    if inside && !corner {
        return Some(Contact { pos: -(n * d - c), normal: n, depth: r - d });
    }
    // the corner between the edges it is outside of
    let j = (0..3).find(|&j| {
        let (f, g) = (flags[j], flags[(j + 1) % 3]);
        (f == -2 || g == -1) && f != g
    })?;
    let p = v[(j + 1) % 3];
    let dv = c - p;
    let d2 = fma(dv.x, dv.x, fma(dv.y, dv.y, dv.z * dv.z));
    if d2 > r * r {
        return None;
    }
    let (dist, normal) = if d2 > 1e-12 {
        let dist = d2.sqrt();
        (dist, dv * recip(dist))
    } else {
        (0.0, n)
    };
    Some(Contact { pos: p, normal, depth: r - dist })
}

/// Who owns an edge (F-38934): the neighbour (-1) when the contact normal faces its normal `nn`
/// more than the polygon's own `ns`; no neighbour: the depth as it is.
fn edge_owner(normal: Vec3, depth: f32, nn: Option<Vec3>, ns: Vec3) -> f32 {
    let Some(nn) = nn else { return depth };
    let a = fma(normal.y, nn.y, fma(normal.z, nn.z, normal.x * nn.x));
    let b = fma(normal.z, ns.z, fma(normal.x, ns.x, normal.y * ns.y));
    if a > b { -1.0 } else { depth }
}

// The capsule against a polygon: F-43031's undirected branch (the query's byte +212 = 0) and its callees, written
// out operation for operation (vector multiply-adds rounded twice, the vector unit's sum of three products, its
// exact reciprocal / reciprocal square root estimates, the scalar unit's fused dots in the original's orders).
// Rules: docs/reference/people/traffic_drive_rule.md 9.6; checked to the bit against the original's code run whole
// offline (`capsule_polygon_matches_the_original`).

/// The neighbour across one edge as the edge owner F-38934 meets it: its normal (as the bound forms it; past the
/// bound's count (0, 0, 1)) and, for a polygon within the count, the x of its vertex at record +4 - the owner fetches
/// it into the stack word the capsule's line test may read later (F-89261's stale parameter).
#[derive(Clone, Copy, Debug)]
pub struct Beside {
    pub normal: Vec3,
    pub v0x: Option<f32>,
}

/// What F-43031 (undirected) leaves for the result list: the depth f31 (-1 when nothing touched), the place along
/// the capsule f24 (0 at its start, 1 at its end; the segment test's t), the point v126 (the capsule's direction
/// when nothing touched) and the contact normal v125 (meaningful only for a depth above 0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CapsuleHit {
    pub depth: f32,
    pub t: f32,
    pub point: Vec3,
    pub normal: Vec3,
}

/// 1 / x as the vector unit's estimate (the recomp's exact division), no Newton step.
fn vrefp(x: f32) -> f32 {
    1.0 / x
}

/// num / den clamped to [0, 1] as the original forms it: num x (1 / den), 1 from 1 up (vsel), then 0 where num is
/// not at least 0 (vand with the compare's mask: NaN gives 0).
fn clamp_param(num: f32, den: f32) -> f32 {
    let q = num * vrefp(den);
    let c = if q >= 1.0 { 1.0 } else { q };
    if num >= 0.0 { c } else { 0.0 }
}

/// F-33142 as F-44891 calls it (no out pointer): the segment `a` .. `b` (direction `d`) against the triangle;
/// (code, dA, dB, t): 3 both ends on one side, 4 t out of [0, 1], 1 the determinant under 7.63e-6, 5 / 6 outside
/// the first / second weight, 0 a crossing.
fn seg_triangle(a: Vec3, d: Vec3, b: Vec3, n: Vec3, v: &[Vec3; 3]) -> (u32, f32, f32, f32) {
    let w = a - v[0];
    let (e2, e1) = (v[2] - v[0], v[1] - v[0]);
    let da = vdot(n, w);
    let db = vdot(n, b - v[0]);
    let pvec = vcross(d, e2);
    let qvec = vcross(w, e1);
    let det = vdot(pvec, e1);
    let vv = vdot(d, qvec);
    let uu = vdot(w, pvec);
    let t = da * recip(da - db);
    let sign = if det > 0.0 { 1.0 } else { -1.0 };
    let (adet, u, vq) = (det * sign, uu * sign, vv * sign);
    if db * da >= 0.0 {
        return (3, da, db, t);
    }
    if !(t >= 0.0) || t > 1.0 {
        return (4, da, db, t);
    }
    if !(adet >= f32::from_bits(0x3700_0000)) {
        return (1, da, db, t);
    }
    if !(u >= 0.0) || u > adet {
        return (5, da, db, t);
    }
    if !(vq >= 0.0) || vq + u > adet {
        return (6, da, db, t);
    }
    (0, da, db, t)
}

/// F-70182 -> F-44891 (lower bound -1, upper 2): the capsule's axis crossing the triangle: (point, t, depth below the
/// plane of the end that is behind it).
fn segment_crossing(a: Vec3, b: Vec3, n: Vec3, v: &[Vec3; 3]) -> Option<(Vec3, f32, f32)> {
    let d = b - a;
    let (code, da, db, t) = seg_triangle(a, d, b, n, v);
    if code != 0 || (t < -1.0 && t > 2.0) {
        return None;
    }
    let depth = -(if da > 0.0 { db } else { da });
    Some((d * t + a, t, depth))
}

/// F-17804 on a triangle: a ball against a convex polygon, both sides of the plane. (code, point, normal, depth, the
/// edge or corner index): 2 the face, 1 an edge, 0 a corner, 3 none. Each edge's side vector's x goes to the stack
/// word `slot` (its -192, F-65002's +128).
fn ball_ngon(c: Vec3, r: f32, v: &[Vec3; 3], n: Vec3, slot: &mut u32) -> (u32, Vec3, Vec3, f32, usize) {
    let none = (3, Vec3::ZERO, Vec3::ZERO, 0.0, 0);
    let w0 = c - v[0];
    let d = fma(n.z, w0.z, fma(n.x, w0.x, n.y * w0.y));
    if d.abs() > r {
        return none;
    }
    let mut inside = true;
    let mut flags = [0i32; 3];
    for i in 0..3 {
        let (a, b) = (v[i], v[(i + 1) % 3]);
        let w = c - a;
        let e = b - a;
        let m = vcross(n, e);
        *slot = m.x.to_bits();
        if !(fma(m.x, w.x, fma(m.y, w.y, m.z * w.z)) < 0.0) {
            flags[i] = 1;
            continue;
        }
        inside = false;
        if fma(e.x, w.x, fma(e.y, w.y, e.z * w.z)) < 0.0 {
            flags[i] = -1;
            continue;
        }
        let u = c - b;
        if fma(u.x, e.x, fma(u.y, e.y, u.z * e.z)) > 0.0 {
            flags[i] = -2;
            continue;
        }
        if !(fma(n.z, w.z, fma(n.x, w.x, n.y * w.y)) > 0.0) {
            continue;
        }
        // the edge
        let ww = fma(w.x, w.x, fma(w.y, w.y, w.z * w.z));
        let uu = fma(u.x, u.x, fma(u.y, u.y, u.z * u.z));
        let ee = fma(e.x, e.x, fma(e.y, e.y, e.z * e.z));
        let inv = 1.0 / ee;
        let dw = ww - uu;
        let d2 = fnms(dw * dw, inv, fms(uu + ww, 2.0, ee)) * 0.25;
        if d2 > r * r {
            return none;
        }
        let dist = if d2 > 0.0 { d2.sqrt() } else { 0.0 };
        let s2 = (ww - d2) * inv;
        let q = if s2 > 0.0 { e * s2.sqrt() } else { Vec3::ZERO } + a;
        let g = c - q;
        let normal = if dist > f32::from_bits(0x2B8C_BCCC) { g * recip(dist) } else { n };
        return (1, q, normal, r - dist, i);
    }
    if inside {
        return (2, -(n * d - c), n, r - d, 0);
    }
    let Some(j) = (0..3).find(|&j| {
        let (f, g) = (flags[j], flags[(j + 1) % 3]);
        (f == -2 || g == -1) && f != g
    }) else {
        return none;
    };
    let k = (j + 1) % 3;
    let p = v[k];
    let dv = c - p;
    let d2 = fma(dv.z, dv.z, fma(dv.x, dv.x, dv.y * dv.y));
    if d2 > r * r {
        return none;
    }
    let (dist, normal) = if d2 > f32::from_bits(0x2B8C_BCCC) {
        let dist = d2.sqrt();
        (dist, dv * recip(dist))
    } else {
        (0.0, n)
    };
    (0, p, normal, r - dist, k)
}

/// F-43156 (no out pointer): the distance of `w` from the line along `dir`, signed as the original's: the unit
/// perpendicular (dir x w) x dir by a double-precision 1 / sqrt with two Newton steps, else -w's unit, else x.
fn line_dist(w: Vec3, dir: Vec3) -> f32 {
    let eps = f32::from_bits(0x2B8C_BCCC);
    let rs = |l2: f32| {
        let l = l2 as f64;
        let x0 = (1.0 / l.sqrt()) as f32 as f64;
        let f6 = -(x0 * x0).mul_add(l, -1.0);
        let f5 = f6.mul_add(x0 * 0.5, x0);
        let f2 = -(f5 * f5).mul_add(l, -1.0);
        let f0 = f2.mul_add(f5 * 0.5, f5) as f32;
        if -l2 >= 0.0 { 0.0 } else { f0 }
    };
    let len2 = |u: Vec3| fma(u.x, u.x, fma(u.y, u.y, u.z * u.z));
    let c2 = vcross(vcross(w, dir), dir);
    let u = if len2(c2) > eps {
        c2 * rs(len2(c2))
    } else {
        let nw = Vec3::ZERO - w;
        if len2(nw) > eps { nw * rs(len2(nw)) } else { Vec3::X }
    };
    -fma(w.x, u.x, fma(w.y, u.y, w.z * u.z))
}

/// F-68765: the distance of the point `w` from the line through 0 along `d` and the unit towards it (the
/// perpendicular (w x d) x d, else -w's unit, else x; lengths from 1e-12).
fn point_line(w: Vec3, d: Vec3) -> (f32, Vec3) {
    let eps = f32::from_bits(0x2B8C_BCCC);
    let nw = Vec3::ZERO - w;
    let ww = vdot(nw, nw);
    let c2 = vcross(vcross(w, d), d);
    let cc = vdot(c2, c2);
    let alt = if ww > eps { nw * rsqrt0(ww) } else { Vec3::X };
    let u = if cc > eps { c2 * rsqrt0(cc) } else { alt };
    (0.0 - vdot(w, u), u)
}

/// F-84741: the same for the point `a` - `p` from the line along `d`, without the guards.
fn point_line_plain(p: Vec3, a: Vec3, d: Vec3) -> (f32, Vec3) {
    let w = a - p;
    let c2 = vcross(vcross(w, d), d);
    let u = c2 * rsqrt0(vdot(c2, c2));
    (0.0 - vdot(w, u), u)
}

/// F-96822: the separation of the edge line (p, e) and the capsule's axis (a, d) and the unit from the axis to the
/// edge: the common perpendicular's (|C|^2 at least |e|^2 |d|^2 x 0.0012217 and above 0), else a point to a line.
fn line_separation(p: Vec3, e: Vec3, a: Vec3, d: Vec3) -> (f32, Vec3) {
    let dd = vdot(d, d);
    let ee = vdot(e, e);
    let c = vcross(d, e);
    let cc = vdot(c, c);
    if !(cc >= ee * dd * f32::from_bits(0x3AA0_2173)) {
        return point_line_plain(p, a, d);
    }
    if !(cc > 0.0) {
        if ee == 0.0 {
            return point_line(a - p, d);
        }
        let (s, u) = point_line(p - a, e);
        return (s, Vec3::ZERO - u);
    }
    let cn = c * rsqrt0(cc);
    let s = vdot(p - a, cn);
    if s >= 0.0 { (s, cn) } else { (0.0 - s, Vec3::ZERO - cn) }
}

/// F-89261: the lines' closest parameters, s on the edge p + e s, t on the axis a + d t; whether both lie strictly
/// inside (0, 1) (integer compares of the bits). With the axis shorter than 1e-6 t is 0.5 and s is not written: the
/// stack word `slot` holds what earlier calls left there.
fn line_params(p: Vec3, e: Vec3, a: Vec3, d: Vec3, slot: &mut u32) -> (bool, f32) {
    let eps = f32::from_bits(0x2B8C_BCCC);
    let dd = vdot(d, d);
    let t = if dd > eps {
        let inv = vrefp(dd);
        let k = vdot(d, e) * inv;
        let e2 = -(d * k - e);
        let ee2 = vdot(e2, e2);
        let s = if ee2 > eps {
            let w = p - a;
            let w2 = -(d * (vdot(d, w) * inv) - w);
            (0.0 - vdot(e2, w2)) * vrefp(ee2)
        } else {
            0.5
        };
        *slot = s.to_bits();
        let x = e * s + p;
        clamp_param(0.0 - vdot(a - x, d), dd)
    } else {
        0.5
    };
    let inner = |b: u32| (b as i32) > 0 && (b as i32) < 0x3F80_0000;
    (inner(*slot) && inner(t.to_bits()), t)
}

/// F-45690: the closest points of the edge segment p .. p + e and the axis segment a .. a + d: (inside both, s, t);
/// past an end the original's table of the clamped ends' parameters (exactly 0, exactly 1, between) and the
/// distances F-43156 gives.
fn segment_params(p: Vec3, e: Vec3, a: Vec3, d: Vec3, slot: &mut u32) -> (bool, f32, f32) {
    let (inner, t) = line_params(p, e, a, d, slot);
    if inner {
        return (true, f32::from_bits(*slot), t);
    }
    let q = e + p;
    let b = d + a;
    let dd = vdot(d, d);
    let tp = clamp_param(0.0 - vdot(a - p, d), dd);
    let tq = clamp_param(0.0 - vdot(a - q, d), dd);
    let ee = vdot(e, e);
    let s_of = |base: Vec3| clamp_param(0.0 - vdot(base, e), ee);
    let (pa, pb) = (p - a, p - b);
    let zero = |x: f32| x.to_bits() & 0x7FFF_FFFF == 0;
    let one = |x: f32| x.to_bits() == 0x3F80_0000;
    #[cfg(test)]
    {
        let class = |x: f32| if zero(x) { 0 } else if one(x) { 1 } else { 2 };
        tests::END_CASES[3 * class(tp) + class(tq)].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    let (s, t) = if zero(tp) {
        if zero(tq) {
            (s_of(pa), 0.0)
        } else if one(tq) {
            if !(line_dist(pa, e) < line_dist(pb, e)) { (s_of(pb), 1.0) } else { (s_of(pa), 0.0) }
        } else if !(line_dist(pa, e) < line_dist(a - q, d)) {
            (1.0, tq)
        } else {
            (s_of(pa), 0.0)
        }
    } else if one(tp) {
        if zero(tq) {
            if !(line_dist(pa, e) < line_dist(pb, e)) { (s_of(pb), 1.0) } else { (s_of(pa), 0.0) }
        } else if one(tq) {
            (s_of(pb), 1.0)
        } else if !(line_dist(pb, e) < line_dist(a - q, d)) {
            (1.0, tq)
        } else {
            (s_of(pb), 1.0)
        }
    } else if zero(tq) {
        if !(line_dist(pa, e) < line_dist(a - p, d)) { (0.0, tp) } else { (s_of(pa), 0.0) }
    } else if one(tq) {
        if !(line_dist(pb, e) < line_dist(a - p, d)) { (0.0, tp) } else { (s_of(pb), 1.0) }
    } else if line_dist(a - p, d) < line_dist(a - q, d) {
        (0.0, tp)
    } else {
        (1.0, tq)
    };
    // (written through the s pointer: the stack word)
    *slot = s.to_bits();
    (false, s, t)
}

/// F-65002: the capsule a .. b (radius r) against the edge p .. q: (code, depth, normal, point, t); 3 none, 4 the
/// closest point past an end of the axis (left to the end balls), 1 the two segments' inner points, 0 an end of
/// the edge. A corner touch writes -distance to the stack word `slot`.
fn edge_capsule(a: Vec3, b: Vec3, p: Vec3, q: Vec3, r: f32, slot: &mut u32) -> (u32, f32, Vec3, Vec3, f32) {
    let none = (3, 0.0, Vec3::ZERO, Vec3::ZERO, 0.0);
    let d = b - a;
    let e = q - p;
    let (sep, towards) = line_separation(p, e, a, d);
    if sep > r {
        return none;
    }
    let (inner, s, t) = segment_params(p, e, a, d, slot);
    if inner {
        return (1, r - sep, Vec3::ZERO - towards, d * t + a, t);
    }
    if !(t > 0.0 && t < 1.0) {
        return if !(t < 0.5) { (4, 0.0, Vec3::ZERO, b, 1.0) } else { (4, 0.0, Vec3::ZERO, a, 0.0) };
    }
    let second = !(s < 0.5);
    let corner = if second { q } else { p };
    let point = d * t + a;
    let cv = corner - point;
    let d2 = fma(cv.z, cv.z, fma(cv.x, cv.x, cv.y * cv.y));
    if d2 > r * r {
        return none;
    }
    if d2 > 0.0 {
        let dist = d2.sqrt();
        *slot = (-dist).to_bits();
        return (0, r - dist, cv * recip(-dist), point, t);
    }
    let nv = if second { Vec3::ZERO - (p - q) } else { p - q };
    (0, r, nv * rsqrt0(vdot(nv, nv)), point, t)
}

/// F-38934 as the capsule test calls it: the neighbour's vertex fetch into the stack word, then the owner rule.
fn owner_at(normal: Vec3, depth: f32, beside: Option<Beside>, own: Vec3, slot: &mut u32) -> f32 {
    if let Some(Beside { v0x: Some(x), .. }) = beside {
        *slot = x.to_bits();
    }
    edge_owner(normal, depth, beside.map(|b| b.normal), own)
}

/// A capsule from `a` to `b` of radius `r` against one polygon, undirected (F-43031 with byte +212 = 0): the axis
/// crossing the face (depth r + the depth of the end behind the plane), else the deeper of the end balls (F-17804, an
/// edge contact through its owner F-38934; the end ball at `b` gives t = 1), and only when neither touched (depth
/// below 0) each edge's capsule test (F-65002), the last edge that touches deciding through its owner. `n` the
/// polygon's normal as the bound forms it, `own` the bound's normal of the query's polygon index, `beside(e)` the
/// neighbour across edge e (None: none). `slot` is the stack word F-44891 saves the caller's f30 in (its high word on
/// entry) and that the capsule's line test reads when the axis has no length; on return as the original leaves it.
#[allow(clippy::too_many_arguments)]
pub fn capsule_polygon(a: Vec3, b: Vec3, r: f32, v: &[Vec3; 3], n: Vec3, own: Vec3, beside: impl Fn(usize) -> Option<Beside>, slot: &mut u32) -> CapsuleHit {
    let mut hit = CapsuleHit { depth: -1.0, t: 0.0, point: b - a, normal: Vec3::ZERO };
    if let Some((point, t, depth)) = segment_crossing(a, b, n, v) {
        hit = CapsuleHit { depth: r + depth, t, point, normal: n };
    }
    for (end, t) in [(a, 0.0), (b, 1.0)] {
        let (code, point, normal, mut depth, i) = ball_ngon(end, r, v, n, slot);
        if code == 3 {
            continue;
        }
        if code == 1 {
            depth = owner_at(normal, depth, beside(i), own, slot);
        }
        if depth > hit.depth {
            hit = CapsuleHit { depth, t, point, normal };
        }
    }
    if !(hit.depth < 0.0) {
        return hit;
    }
    for j in 0..3 {
        let (code, depth, normal, point, t) = edge_capsule(a, b, v[j], v[(j + 1) % 3], r, slot);
        if code == 3 || code == 4 {
            continue;
        }
        let k = r - depth;
        hit = CapsuleHit { depth: owner_at(normal, depth, beside(j), own, slot), t, point: -(normal * k - point), normal };
    }
    hit
}

// The polygon sweep F-43031 (directed) as the original runs it, operation for operation (vector
// multiply-adds rounded twice, the vector unit's three-product sum, the scalar unit's fused dots;
// checked to the bit on 43,280 random sweeps: `polygon_sweep_matches_the_original`).

fn yzx(a: Vec3) -> Vec3 {
    Vec3::new(a.y, a.z, a.x)
}

fn zxy(a: Vec3) -> Vec3 {
    Vec3::new(a.z, a.x, a.y)
}

/// The vector unit's cross product of the sweep (a.yzx b.zxy - a.zxy b.yzx, the last step negated).
fn vcross(a: Vec3, b: Vec3) -> Vec3 {
    -(zxy(a) * yzx(b) - yzx(a) * zxy(b))
}

/// 1 / sqrt with the zero guard (the estimate's infinity becomes 0) and one Newton step.
fn rsqrt0(x: f32) -> f32 {
    let mut y = 1.0 / x.sqrt();
    if y == f32::INFINITY {
        y = 0.0;
    }
    (-(x * (y * y) - 1.0)) * (0.5 * y) + y
}

/// The one-sided crossing of the segment p..q with the triangle (F-25859 -> F-70302): the determinant at least
/// 7.63e-6, both weights in range.
fn crosses(v: &[Vec3; 3], p: Vec3, q: Vec3) -> bool {
    let (e2, e1, seg) = (v[2] - v[0], v[1] - v[0], q - p);
    let c = vcross(seg, e2);
    let det = fma(c.z, e1.z, fma(e1.y, c.y, e1.x * c.x));
    if !(det >= 7.629_394_5e-6) {
        return false;
    }
    let w = p - v[0];
    let u = dot_xyz(w, c);
    if u < 0.0 || u > det {
        return false;
    }
    let vv = dot_xyz(seg, vcross(w, e1));
    vv >= 0.0 && !(vv + u > det)
}

/// The roots of x^2 + b x + c (F-64515): two by the stable form, one where the discriminant is within
/// 1e-6 of the larger coefficient's size, else none.
fn roots(b: f32, c: f32) -> (u32, f32, f32) {
    let disc = b * b - (((c + c) + c) + c);
    let th = 1e-6 * b.abs().max(c.abs());
    if disc > th {
        let sq = disc * rsqrt0(disc);
        let s = if b > 0.0 { sq } else { 0.0 - sq };
        let q = (0.0 - (b + s)) * 0.5;
        return (2, q, c * recip(q));
    }
    if disc >= 0.0 - th {
        return (1, (0.0 - b) * 0.5, 0.0);
    }
    (0, 0.0, 0.0)
}

/// Whether the ball moving from `o` along the unit `dr` first meets the ball round the corner `c`
/// on the side away from the edge `e` (F-55914).
fn meets_corner(o: Vec3, dr: Vec3, rr: f32, c: Vec3, e: Vec3) -> bool {
    let w = c - o;
    let b = vdot(dr, w);
    let perp = -(dr * b - w);
    let h2 = rr - vdot(perp, perp);
    if !(h2 > 0.0) {
        return false;
    }
    let t = b - h2 * rsqrt0(h2);
    if !(t > 0.0) {
        return false;
    }
    let p = dr * t + o;
    vdot(e, c - p) > 0.0
}

/// The ball's touch of the corner `c` (F-43026): its depth (the distance it is carried past the touch, to
/// the end `ep`) when deeper than `best`, the corner and the normal there.
fn corner_hit(best: f32, o: Vec3, dr: Vec3, ep: Vec3, rr: f32, c: Vec3) -> Option<(f32, Vec3, Vec3)> {
    let w = c - o;
    let b = vdot(dr, w);
    if !(b > 0.0) {
        return None;
    }
    let perp = -(dr * b - w);
    let p2 = vdot(perp, perp);
    if p2 >= rr {
        return None;
    }
    let h2 = rr - p2;
    let q = ep - c;
    let h = h2 * rsqrt0(h2);
    let ahead = fma(q.x, dr.x, fma(q.y, dr.y, q.z * dr.z));
    let qq = vdot(q, q);
    let d = if ahead > 0.0 {
        let x = qq - p2;
        x * rsqrt0(x) + h
    } else {
        if qq > rr {
            return None;
        }
        let x = qq - p2;
        h - x * rsqrt0(x)
    };
    if !(d > best) {
        return None;
    }
    let e = -(dr * d - ep) - c;
    Some((d, c, e * rsqrt0(vdot(e, e))))
}

/// The ball (radius `r`, from `o` by `dd`, length `l`, end `ep`) against the edge from `a` by `e`
/// (F-58482): the side of the capsule by its quadratic, else the two corners; (depth, contact,
/// normal) when deeper than `best`. `n` is the polygon's normal.
#[allow(clippy::too_many_arguments)]
fn sweep_edge(best: f32, o: Vec3, dd: Vec3, ep: Vec3, a: Vec3, e: Vec3, r: f32, l: f32, n: Vec3) -> Option<(f32, Vec3, Vec3)> {
    if !(l > 1e-6) {
        return None;
    }
    let rr = r * r;
    let ee = vdot(e, e);
    if ee > 0.0 {
        let de = vdot(dd, e);
        let w = a - o;
        let den = ee * (l * l) - de * de;
        if den.abs() > 1e-6 {
            let (we, ww, dw) = (vdot(w, e), vdot(w, w), vdot(dd, w));
            let rd = recip(den);
            let b = ((we * de - ee * dw) * 2.0) * rd;
            let c = (ee * (ww - rr) - we * we) * rd;
            let (k, r1, r2) = roots(b, c);
            if k > 0 {
                let t = if k == 2 && r1 >= r2 && (r1 < 0.0 || r2 >= 0.0) { r2 } else { r1 };
                if t >= 0.0 && !(t > 1.0) {
                    let dep = (1.0 - t) * l;
                    if dep > best {
                        let ct = dd * t + o;
                        let s = (0.0 - vdot(a - ct, e)) * recip(ee);
                        if s >= 0.0 && !(s > 1.0) {
                            let p = e * s + a;
                            let g = ct - p;
                            let nn = g * rsqrt0(vdot(g, g));
                            if !(vdot(dd, n) >= 0.0) || !(vdot(n, nn) >= 0.0) {
                                return Some((dep, p, nn));
                            }
                            return None;
                        }
                    }
                }
            }
        }
    }
    let dr = dd * recip(l);
    if meets_corner(o, dr, rr, a, e) {
        return corner_hit(best, o, dr, ep, rr, a);
    }
    let b = a + e;
    if meets_corner(o, dr, rr, b, Vec3::ZERO - e) {
        return corner_hit(best, o, dr, ep, rr, b);
    }
    None
}

/// The record of a touch: when (0..1 along the sweep), how deep (the distance carried past it),
/// the contact point and the normal.
#[derive(Clone, Copy, Debug)]
pub struct SweepHit {
    pub t: f32,
    pub depth: f32,
    pub contact: Vec3,
    pub normal: Vec3,
}

/// A ball swept from `o` by `d` against one polygon, directed (F-43031), `n` its normal as the
/// bound forms it (from the area its record holds): only against its front, from a start in front
/// of its plane; the face where the ball's touch of the plane lies over it, else the deepest
/// touch of an edge or corner whose normal does not face along the sweep.
pub fn sweep_polygon_hit(o: Vec3, d: Vec3, r: f32, v: &[Vec3; 3], n: Vec3) -> Option<SweepHit> {
    let dn = vdot(n, d);
    if dn >= 0.0 {
        return None;
    }
    let s = vdot(n, o - v[0]);
    if !(s > 0.0) {
        return None;
    }
    let inv = recip(dn);
    let rinv = r * inv;
    let sinv = (0.0 - s) * inv;
    if sinv > 1.0 - rinv {
        return None;
    }
    let t0 = sinv + rinv;
    if !(t0 >= 0.0) || t0 > 1.0 {
        return None;
    }
    let rr = {
        let f = r * 1.01;
        if f - 0.01 >= 0.0 { f } else { 0.01 }
    };
    let l2 = vdot(-d, -d);
    let l = l2 * rsqrt0(l2);
    let tp = (l - (1.0 - t0) * l) * recip(l);
    let ct = d * tp + o;
    if crosses(v, ct + n * rr, ct - n * rr) {
        let depth = (1.0 - t0) * l;
        return (depth > 0.0).then(|| SweepHit { t: t0, depth, contact: -(n * r - ct), normal: n });
    }
    let ep = o + d;
    let mut best = -1.0f32;
    let mut hit = None;
    for i in 0..3 {
        let Some((dep, p, nn)) = sweep_edge(best, o, d, ep, v[i], v[(i + 1) % 3] - v[i], r, l, n) else { continue };
        if fma(nn.z, d.z, fma(nn.x, d.x, nn.y * d.y)) > 0.0 || !(dep > best) || fma(n.z, d.z, fma(n.x, d.x, n.y * d.y)) > 0.0 {
            continue;
        }
        best = dep;
        hit = Some(SweepHit { t: (l - dep) / l, depth: dep, contact: p, normal: nn });
    }
    hit.filter(|_| best > 0.0)
}

/// The same as (t, normal).
pub fn sweep_polygon(o: Vec3, d: Vec3, r: f32, v: &[Vec3; 3], n: Vec3) -> Option<(f32, Vec3)> {
    sweep_polygon_hit(o, d, r, v, n).map(|h| (h.t, h.normal))
}

impl<'a> CamLevel<'a> {
    /// The city alone.
    pub fn city(city: &'a WorldCollider) -> CamLevel<'a> {
        CamLevel { city, cars: Vec::new() }
    }

    /// A ball's contacts at `c` (F-87651): up to `max`, by the result list's rule; `cars`: the
    /// other cars too (the boom, the event camera's pull-in, the replay's tests), else the city only
    /// (the event camera's deflection and chooser: the fixed objects).
    pub fn contacts(&self, c: Vec3, r: f32, cars: bool, max: usize) -> Vec<Contact> {
        let mut list = List { max, cap: r, held: Vec::new() };
        let ext = Vec3::splat(r);
        // (the edge flags' stack slot, as the previous polygon's call left it)
        let mut flags = [0i32; 3];
        self.city.triangles_of_ball(c - ext, c + ext, Some((c, r)), |m, i| {
            let v = &m.tris[i];
            let n = m.faces.get(i).copied().unwrap_or_else(|| (v[1] - v[0]).cross(v[2] - v[0]).normalize_or_zero());
            let beside = |e: usize| m.beside.get(i).and_then(|b| b[e]).map(|j| m.faces.get(j as usize).copied().unwrap_or(Vec3::Z));
            if let Some(k) = ball_polygon(c, r, v, n, n, beside, &mut flags) {
                list.add(k);
            }
        });
        if cars {
            for o in &self.cars {
                if o.centre.distance(c) > o.radius + r {
                    continue;
                }
                for (i, v) in o.tris.iter().enumerate() {
                    let beside = |e: usize| o.beside[i][e].map(|j| o.normals[j as usize]);
                    if let Some(k) = ball_polygon(c, r, v, o.normals[i], o.normals[i], beside, &mut flags) {
                        list.add(k);
                    }
                }
            }
        }
        list.held.into_iter().map(|h| h.1).collect()
    }

    /// A ball swept from `a` to `b`, directed (F-43031 through F-80278): the first touch over
    /// everything, (t, normal).
    pub fn sweep(&self, a: Vec3, b: Vec3, r: f32, cars: bool) -> Option<(f32, Vec3)> {
        let d = b - a;
        if d.length_squared() < 1e-12 {
            return None;
        }
        let mut best: Option<(f32, Vec3)> = None;
        let mut take = |h: Option<(f32, Vec3)>| {
            if let Some(h) = h {
                if best.map_or(true, |b| h.0 < b.0) {
                    best = Some(h);
                }
            }
        };
        let (mn, mx) = (a.min(b) - Vec3::splat(r), a.max(b) + Vec3::splat(r));
        let ball = ((a + b) * 0.5, d.length() * 0.5 + r);
        self.city.triangles_of_ball(mn, mx, Some(ball), |m, i| {
            let v = &m.tris[i];
            let n = m.faces.get(i).copied().unwrap_or_else(|| (v[1] - v[0]).cross(v[2] - v[0]).normalize_or_zero());
            take(sweep_polygon(a, d, r, v, n));
        });
        if cars {
            for o in &self.cars {
                take(o.sweep(a, b, r));
            }
        }
        best
    }

    /// The line probe (F-20855): one-sided, the nearest hit; the fixed objects only (the city).
    pub fn line(&self, a: Vec3, b: Vec3) -> bool {
        let d = b - a;
        let l = d.length();
        l > 1e-4 && self.city.probe_front(a, d / l, l).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How often F-45690's table met each pair of the axis parameters of the edge's ends (0, 1, between).
    pub(super) static END_CASES: [std::sync::atomic::AtomicUsize; 9] = [const { std::sync::atomic::AtomicUsize::new(0) }; 9];

    const FLOOR: [Vec3; 3] = [Vec3::new(-4.0, 0.0, -4.0), Vec3::new(-4.0, 0.0, 4.0), Vec3::new(4.0, 0.0, -4.0)];

    #[test]
    fn ball_polygon_cases() {
        let n = Vec3::Y;
        // the face, both sides of the plane; behind it the normal is still the face's
        let k = ball_polygon(Vec3::new(-2.0, 0.3, -2.0), 0.5, &FLOOR, n, n, |_| None, &mut [0; 3]).unwrap();
        assert!((k.depth - 0.2).abs() < 1e-6 && k.normal == n && k.pos.y == 0.0);
        let k = ball_polygon(Vec3::new(-2.0, -0.3, -2.0), 0.5, &FLOOR, n, n, |_| None, &mut [0; 3]).unwrap();
        assert!((k.depth - 0.8).abs() < 1e-6 && k.normal == n);
        assert!(ball_polygon(Vec3::new(-2.0, 0.6, -2.0), 0.5, &FLOOR, n, n, |_| None, &mut [0; 3]).is_none());
        // an edge (x = -4) from in front: normal from the edge to the centre
        let k = ball_polygon(Vec3::new(-4.3, 0.3, 0.0), 0.5, &FLOOR, n, n, |_| None, &mut [0; 3]).unwrap();
        assert!((k.depth - (0.5 - 0.18f32.sqrt())).abs() < 1e-5 && (k.pos - Vec3::new(-4.0, 0.0, 0.0)).length() < 1e-5, "{k:?}");
        // the same edge, behind the plane: no edge contact, and outside the face: none
        assert!(ball_polygon(Vec3::new(-4.3, -0.3, 0.0), 0.5, &FLOOR, n, n, |_| None, &mut [0; 3]).is_none());
        // the neighbour across the edge owns it when the contact faces it more
        assert!(ball_polygon(Vec3::new(-4.3, 0.3, 0.0), 0.5, &FLOOR, n, n, |e| (e == 0).then_some(Vec3::NEG_X), &mut [0; 3]).is_none_or(|k| k.pos == FLOOR[0] || k.pos == FLOOR[1]));
        // a corner
        let k = ball_polygon(Vec3::new(-4.2, 0.1, -4.2), 0.5, &FLOOR, n, n, |_| None, &mut [0; 3]).unwrap();
        assert!(k.pos == FLOOR[0] && (k.depth - (0.5 - 0.09f32.sqrt())).abs() < 1e-5, "{k:?}");
    }

    #[test]
    fn list_keeps_the_deepest() {
        let mut l = List { max: 2, cap: 1.0, held: Vec::new() };
        let k = |d: f32| Contact { pos: Vec3::ZERO, normal: Vec3::Y, depth: d };
        for d in [0.2, 0.5, 1.3, 0.4, 0.1] {
            l.add(k(d));
        }
        // 1.3 past the cap scores -0.3: dropped for 0.4
        assert_eq!(l.held.iter().map(|h| h.1.depth).collect::<Vec<_>>(), vec![0.4, 0.5]);
    }

    #[test]
    fn sweep_polygon_cases() {
        let n = Vec3::Y;
        // the face: the touch when the centre is r above the plane
        let (t, nr) = sweep_polygon(Vec3::new(-2.0, 2.0, -2.0), Vec3::new(0.0, -4.0, 0.0), 0.5, &FLOOR, n).unwrap();
        assert!((t - 0.375).abs() < 1e-6 && nr == n);
        // from behind, or moving along its front: none
        assert!(sweep_polygon(Vec3::new(-2.0, -2.0, -2.0), Vec3::new(0.0, 4.0, 0.0), 0.5, &FLOOR, n).is_none());
        // a start within r of the plane: the polygon is left out
        assert!(sweep_polygon(Vec3::new(-2.0, 0.4, -2.0), Vec3::new(0.0, -1.0, 0.0), 0.5, &FLOOR, n).is_none());
        // past the x = -4 edge: the capsule's touch
        let (t, nr) = sweep_polygon(Vec3::new(-4.3, 2.0, 0.0), Vec3::new(0.0, -4.0, 0.0), 0.5, &FLOOR, n).unwrap();
        let y = (0.25f32 - 0.09).sqrt();
        assert!((t - (2.0 - y) / 4.0).abs() < 1e-5 && (nr - Vec3::new(-0.3, y, 0.0) / 0.5).length() < 1e-5, "{t} {nr}");
    }

    /// F-82650 with F-38934 run offline on the original's code (tools/ball_polygon_reference.py ->
    /// testdata/ball_polygon_ref.txt): per line six vertices (the polygon's, then the apex of the
    /// neighbour folded about each edge), the centre, the radius, the area word, the three
    /// neighbour indices (1 .. 3, 0xFFFF none, 7 past the count 4) and the neighbours' area words,
    /// the query's polygon index (5: past the count), the material mask and byte, the edge flags'
    /// stale words, whether the list gives a record; then the original's result, the list's call
    /// (depth, cap, point), the record and the flags after. 4,000 cases, every word equal.
    #[test]
    fn ball_polygon_matches_the_original() {
        use super::super::gjk::polygon_normal;
        let u = |h: &str| u32::from_str_radix(&h.replace('_', ""), 16).unwrap();
        let mut seen = [0usize; 4];
        for (k, line) in include_str!("../testdata/ball_polygon_ref.txt").lines().enumerate() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = i.split_whitespace().map(u).collect();
            let x: Vec<u32> = o.split_whitespace().map(u).collect();
            let f = |i: usize| f32::from_bits(w[i]);
            let p = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            let vs: Vec<Vec3> = (0..6).map(|j| p(3 * j)).collect();
            let v = [vs[0], vs[1], vs[2]];
            let (c, r, area) = (p(18), f(21), f(22));
            let n = polygon_normal(&v, area);
            let own = if w[29] == 0 { n } else { Vec3::Z };
            let beside = |e: usize| match w[23 + e] {
                0xFFFF => None,
                j @ 1..=3 => {
                    let j = j as usize;
                    Some(polygon_normal(&[vs[j % 3], vs[j - 1], vs[2 + j]], f(25 + j)))
                }
                _ => Some(Vec3::Z),
            };
            let mut flags = [w[32] as i32, w[33] as i32, w[34] as i32];
            let got = ball_polygon(c, r, &v, n, own, beside, &mut flags);
            let mat = w[31] << 8;
            let bits = |x: Vec3| [x.x.to_bits(), x.y.to_bits(), x.z.to_bits()];
            let (ret, called) = match got {
                Some(h) if mat & w[30] == 0 => {
                    assert_eq!([x[2], x[3]], [h.depth.to_bits(), r.to_bits()], "line {k}: depth, cap");
                    assert_eq!(&x[4..7], &bits(h.pos), "line {k}: point");
                    if w[35] != 0 {
                        assert_eq!(&x[8..11], &bits(h.pos), "line {k}: contact");
                        assert_eq!(&x[11..14], &bits(h.normal), "line {k}: normal");
                        assert_eq!(&x[14..17], &bits(n), "line {k}: polygon normal");
                        assert_eq!(&x[17..20], &[0, h.depth.to_bits(), mat], "line {k}: t, depth, material");
                        assert_eq!(&x[20..24], &[w[29], 2, (h.depth < r) as u32, 0], "line {k}: index, kind, below, +82");
                        assert_eq!(x[7], 0x1234, "line {k}: instance");
                    }
                    seen[if h.normal == n { 0 } else if v.contains(&h.pos) { 2 } else { 1 }] += 1;
                    (if w[35] != 0 { 1 } else { (h.depth > 0.0) as i32 }, 1)
                }
                _ => {
                    seen[3] += 1;
                    (0, 0)
                }
            };
            assert_eq!([x[0] as i32, x[1] as i32], [ret, called], "line {k}: result, list call");
            assert_eq!(&x[24..27], &flags.map(|f| f as u32), "line {k}: flags after");
        }
        // (face, edge, corner, none or left out by the material)
        assert_eq!(seen, [1564, 311, 305, 1820]);
    }

    /// F-43031's undirected branch run offline on the original's code with every callee whole
    /// (tools/capsule_polygon_reference.py -> testdata/capsule_polygon_ref.txt): per line six vertices (the polygon's,
    /// then the apex of the neighbour folded about each edge), the capsule's ends and radius, the area word, the three
    /// neighbour indices (1 .. 3, 0xFFFF none, 7 past the count 4) and their area words, the query's polygon index (5:
    /// past the count), the material mask and byte, the caller's f30 (two words), whether the list gives a record;
    /// then the original's result, the list's call (depth, cap, point), the record and the stale stack word after.
    #[test]
    fn capsule_polygon_matches_the_original() {
        use super::super::gjk::polygon_normal;
        let u = |h: &str| u32::from_str_radix(&h.replace('_', ""), 16).unwrap();
        // (axis through the face, an end ball, the edge tests: touched / not; capsules without length)
        let mut seen = [0usize; 5];
        for (k, line) in include_str!("../testdata/capsule_polygon_ref.txt").lines().enumerate() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = i.split_whitespace().map(u).collect();
            let x: Vec<u32> = o.split_whitespace().map(u).collect();
            let f = |i: usize| f32::from_bits(w[i]);
            let p = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            let vs: Vec<Vec3> = (0..6).map(|j| p(3 * j)).collect();
            let v = [vs[0], vs[1], vs[2]];
            let (a, b, r, area) = (p(18), p(21), f(24), f(25));
            let n = polygon_normal(&v, area);
            let own = if w[32] == 0 { n } else { Vec3::Z };
            let beside = |e: usize| match w[26 + e] {
                0xFFFF => None,
                j @ 1..=3 => {
                    let j = j as usize;
                    Some(Beside { normal: polygon_normal(&[vs[j % 3], vs[j - 1], vs[2 + j]], f(28 + j)), v0x: Some(vs[j % 3].x) })
                }
                _ => Some(Beside { normal: Vec3::Z, v0x: None }),
            };
            let mut slot = w[35];
            let h = capsule_polygon(a, b, r, &v, n, own, beside, &mut slot);
            let bits = |x: Vec3| [x.x.to_bits(), x.y.to_bits(), x.z.to_bits()];
            let mat = w[34] << 8;
            if mat & w[33] != 0 {
                assert_eq!(&x[0..2], &[0, 0], "line {k}: material");
            } else {
                assert_eq!([x[0], x[1]], [(h.depth > 0.0) as u32, 1], "line {k}: result, list call");
                assert_eq!([x[2], x[3]], [h.depth.to_bits(), 0], "line {k}: depth, cap");
                assert_eq!(&x[4..7], &bits(h.point), "line {k}: point");
                if w[37] != 0 && h.depth > 0.0 {
                    assert_eq!(x[7], 0x1234, "line {k}: instance");
                    assert_eq!(&x[8..11], &bits(h.point), "line {k}: contact");
                    assert_eq!(&x[11..14], &bits(h.normal), "line {k}: normal");
                    assert_eq!(&x[14..17], &bits(n), "line {k}: polygon normal");
                    assert_eq!(&x[17..20], &[h.t.to_bits(), h.depth.to_bits(), mat], "line {k}: t, depth, material");
                    assert_eq!(&x[20..24], &[w[32], 2, 1, 0], "line {k}: index, kind, +80, +82");
                }
            }
            assert_eq!(x[24], slot, "line {k}: stale word");
            let mut s = 0;
            if segment_crossing(a, b, n, &v).is_some() {
                seen[0] += 1;
            } else if [a, b].iter().any(|&c| {
                let (code, _, _, d, _) = ball_ngon(c, r, &v, n, &mut s);
                code != 3 && d >= 0.0
            }) {
                seen[1] += 1;
            } else {
                seen[if h.depth > 0.0 { 2 } else { 3 }] += 1;
            }
            if !(vdot(b - a, b - a) > f32::from_bits(0x2B8C_BCCC)) {
                seen[4] += 1;
            }
        }
        let ends: Vec<usize> = END_CASES.iter().map(|c| c.load(std::sync::atomic::Ordering::Relaxed)).collect();
        assert_eq!((seen, ends), ([506, 1430, 518, 1546, 283], vec![345, 90, 158, 89, 379, 135, 147, 152, 932]));
    }

    /// F-43031 (directed) run offline on the original's code (work/cameras2/sweep/sweep_ref.py):
    /// per line the polygon, start, end, radius and the area word its record holds, then the
    /// original's result, alloc call, depth and record (+16 contact, +32 normal, +48, +64 t, +68
    /// depth). 5,629 random sweeps: 151 face touches, 340 edge or corner touches.
    #[test]
    fn polygon_sweep_matches_the_original() {
        let f = |h: &str| f32::from_bits(u32::from_str_radix(h, 16).unwrap());
        let (mut face, mut edge, mut none) = (0, 0, 0);
        for (k, line) in include_str!("../testdata/polygon_sweep_ref.txt").lines().enumerate() {
            let w: Vec<&str> = line.split_whitespace().collect();
            let p = |i: usize| Vec3::new(f(w[i]), f(w[i + 1]), f(w[i + 2]));
            let v = [p(0), p(3), p(6)];
            let (o, e, r, area) = (p(9), p(12), f(w[15]), f(w[16]));
            let n = super::super::gjk::polygon_normal(&v, area);
            let got = sweep_polygon_hit(o, e - o, r, &v, n);
            let bits = |x: Vec3| [x.x.to_bits(), x.y.to_bits(), x.z.to_bits()];
            let u = |i: usize| u32::from_str_radix(w[17 + i], 16).unwrap();
            match got {
                None => {
                    assert_eq!(w[17], "0", "line {k}: the original touches");
                    none += 1;
                }
                Some(h) => {
                    assert_eq!(w[17], "1", "line {k}: the original does not touch");
                    assert_eq!(bits(h.contact), [u(3), u(4), u(5)], "line {k} contact");
                    assert_eq!(bits(h.normal), [u(6), u(7), u(8)], "line {k} normal");
                    assert_eq!([h.t.to_bits(), h.depth.to_bits()], [u(12), u(13)], "line {k} t, depth");
                    if h.normal == n { face += 1 } else { edge += 1 }
                }
            }
        }
        assert_eq!((face, edge, none), (151, 340, 5138));
    }
}
