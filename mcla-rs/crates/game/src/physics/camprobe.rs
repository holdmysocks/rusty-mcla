//! The level as the cameras' probes see it (rules in `docs/reference/cameras/probe_rule.md`): the
//! city and the other cars, a ball's contacts at a point (F-87651 -> F-82650, up to `max` kept by
//! the result list's score, F-89028), a ball swept along a segment (F-43031, directed) and the
//! one-sided line probe (F-20855). The camera's own car is left out by its identity.

use glam::{Mat3, Quat, Vec3};

use super::collision::WorldCollider;
use super::gjk;
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
        tests::END_CASES.with(|c| c.borrow_mut()[3 * class(tp) + class(tq)] += 1);
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

// A composite bound's parts as the capsule query steps through them (F-35029, type 12; rules in
// docs/reference/people/traffic_drive_rule.md 9.7): the query moved into the part's frame, the part dropped when the
// capsule misses its box, and the query moved back after the part - by the matrix, not restored, so each part after
// the first meets the capsule as the rounding of the way there and back left it. Checked to the bit against the
// original's code run offline (`part_prune_matches_the_original`).

/// The capsule query in a part's frame and its box (F-66128): each end less the part's place dotted with the rows `m[0..3]`
/// (the vector unit's three-product sum), the box the ends' smaller values less `r` and larger plus `r`. (a, b, min, max).
pub fn part_frame(a: Vec3, b: Vec3, r: f32, m: &[Vec3; 4]) -> (Vec3, Vec3, Vec3, Vec3) {
    let to = |p: Vec3| {
        let w = p - m[3];
        Vec3::new(vdot(m[0], w), vdot(m[1], w), vdot(m[2], w))
    };
    let (a, b) = (to(a), to(b));
    (a, b, a.min(b) - Vec3::splat(r), a.max(b) + Vec3::splat(r))
}

/// The capsule query back out of the part's frame (F-15412 -> F-53488): each end as (x, y, z, 1) times the matrix's
/// columns, the vector unit's four-product sum (x m0 + y m1) + (z m2 + place); the box again as `part_frame`'s.
pub fn part_unframe(a: Vec3, b: Vec3, r: f32, m: &[Vec3; 4]) -> (Vec3, Vec3, Vec3, Vec3) {
    let (a, b) = (out_of(a, m), out_of(b, m));
    (a, b, a.min(b) - Vec3::splat(r), a.max(b) + Vec3::splat(r))
}

/// A point of a frame in the frame's parent: (x, y, z, 1) times the columns of rows `m[0..3]` and place `m[3]`, the
/// vector unit's four-product sum (x m0 + y m1) + (z m2 + place) (F-53488, F-62425).
fn out_of(p: Vec3, m: &[Vec3; 4]) -> Vec3 {
    let col = |k: usize| (p.x * m[0][k] + p.y * m[1][k]) + (p.z * m[2][k] + m[3][k]);
    Vec3::new(col(0), col(1), col(2))
}

/// The level query's volume as its culler holds it (the query drivers' set-up): the sphere query's ball (F-36470: its
/// centre and radius, type 0; a radius not above 0 (a NaN too) gives the point, type 5, its centre alone) or the swept
/// capsule's (F-21683: the start, the unit direction - the rsqrt estimate with its zero guard and one Newton step - the
/// length and the radius, type 3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CullVolume {
    Ball { centre: Vec3, r: f32 },
    Capsule { from: Vec3, dir: Vec3, len: f32, r: f32 },
    Point { centre: Vec3 },
}

impl CullVolume {
    /// F-36470: the sphere query's ball at `c` of radius `r`; a radius not above 0 is the point at `c`.
    pub fn ball(c: Vec3, r: f32) -> CullVolume {
        if r > 0.0 { CullVolume::Ball { centre: c, r } } else { CullVolume::Point { centre: c } }
    }

    /// F-21683: the capsule from `a` to `b` of radius `r` as the sweep's culler takes it.
    pub fn capsule(a: Vec3, b: Vec3, r: f32) -> CullVolume {
        let d = b - a;
        let l2 = fma(d.x, d.x, fma(d.z, d.z, d.y * d.y));
        let len = if l2 > 0.0 { l2.sqrt() } else { 0.0 };
        CullVolume::Capsule { from: a, dir: d * rsqrt0(vdot(d, d)), len, r }
    }
}

/// What the culler asks of a level object before its bound is tested (F-62425, the culler's +100 = 0 -> F-92940): the
/// object's state bit among the query's states, its archetype's type among the include mask and its include word among
/// the type flags, its type not among the excluded types (always 0 for the queries here); then its bound's sphere - the
/// bound's centre (+48) through the instance matrix (rows and place), its radius (+8) - meeting the volume: a ball when
/// (R + r)^2 > |c - centre|^2, a capsule when (R + r)^2 > |q - centre|^2 for q the point of the axis nearest the centre
/// (dir t + from with t the projection, 0 where not above 0, the length past it; products and sums rounded apart), a point
/// when r^2 > |q - centre|^2.
pub fn instance_culled_in(vol: &CullVolume, m: &[Vec3; 4], centre: Vec3, radius: f32) -> bool {
    let c = out_of(centre, m);
    match *vol {
        CullVolume::Ball { centre: q, r } => {
            let d = q - c;
            let s = r + radius;
            s * s > vdot(d, d)
        }
        CullVolume::Capsule { from, dir, len, r } => {
            let t = vdot(c - from, dir);
            let t = if t > 0.0 { if t < len { t } else { len } } else { 0.0 };
            let d = (dir * t + from) - c;
            let s = radius + r;
            s * s > vdot(d, d)
        }
        CullVolume::Point { centre: q } => {
            let d = q - c;
            radius * radius > vdot(d, d)
        }
    }
}

/// The culler's masks (F-92940): the object's state bit (1 active, 2 inactive, 4 fixed) in `states`, the archetype's type
/// word in `include` and its include word in `types`, the type word not in `exclude`.
pub fn instance_masks_pass(states: u32, state_bit: u32, include: u32, types: u32, exclude: u32, arch_type: u32, arch_include: u32) -> bool {
    states & state_bit != 0 && arch_type & include != 0 && arch_include & types != 0 && arch_type & exclude == 0
}

/// A bound that is not made of polygons, in its own frame: a sphere (type 0: its centroid +48 and radius +8), a capsule
/// (type 1: radius +128, length +144, along y about the centroid) or a box (type 3: size +208 about the centroid).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Primitive {
    Sphere { centre: Vec3, r: f32 },
    Capsule { centre: Vec3, r: f32, len: f32 },
    Box { centre: Vec3, size: Vec3 },
}

/// What a primitive test leaves for the result list: the depth f31 (-1 for a capsule out of reach), the contact normal
/// v127, the part index r31 the bound's material is asked for and the record names (+76; a capsule 0 its lower cap, 1 the
/// upper, 2 the side; a box the vertex 0 .. 7 or the face 0 .. 5, +x -x +y -y +z -z) and the kind r29 (+78; a box's corner
/// 0, else 2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrimContact {
    pub depth: f32,
    pub normal: Vec3,
    pub part: u16,
    pub kind: u16,
}

/// The box's corners as the original's table holds them (0x828123F0: +-0.5) and its faces' normals (0x82812470); a box
/// edge's two faces (0x8207FC20); the classifier's (feature kind, index) for the code x + 3 y + 9 z of the three axes'
/// regions (0 at or below -h, 1 inside, 2 at or above h) (F-25936's table).
const BOX_CORNER: [[f32; 3]; 12] = [
    [0.5, 0.5, 0.5], [-0.5, 0.5, 0.5], [-0.5, -0.5, 0.5], [0.5, -0.5, 0.5],
    [0.5, 0.5, -0.5], [-0.5, 0.5, -0.5], [-0.5, -0.5, -0.5], [0.5, -0.5, -0.5],
    // (an edge's index past the corners reads on into the faces' table)
    [1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, -1.0, 0.0],
];
const BOX_FACE: [Vec3; 6] = [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z];
const BOX_EDGE: [(usize, usize); 12] = [(2, 4), (1, 4), (3, 4), (0, 4), (2, 5), (1, 5), (3, 5), (0, 5), (0, 2), (1, 2), (1, 3), (0, 3)];
const BOX_FEATURE: [(i32, usize); 27] = [
    (0, 6), (1, 6), (0, 7), (1, 5), (2, 5), (1, 7), (0, 5), (1, 4), (0, 4),
    (1, 10), (2, 3), (1, 11), (2, 1), (-1, 0), (2, 0), (1, 9), (2, 2), (1, 8),
    (0, 2), (1, 2), (0, 3), (1, 1), (2, 4), (1, 3), (0, 1), (1, 0), (0, 0),
];

/// F-25936: the box feature (kind, index) of the point `d` against the box of half sizes `half` about 0 - per axis 0 at
/// or below -h (and a NaN), 1 inside, 2 at or above h, the code x + 3 y + 9 z read in the classifier's table.
fn box_feature(d: Vec3, half: Vec3) -> (i32, usize) {
    let region = |p: f32, h: f32| if !(p > -h) { 0 } else if p < h { 1 } else { 2 };
    BOX_FEATURE[region(d.x, half.x) + 3 * region(d.y, half.y) + 9 * region(d.z, half.z)]
}

/// v x (1 / dist) as the vector unit gives it: the estimate (the recomp's exact division) and one Newton step, the
/// products and sums rounded apart.
fn unit_by(v: Vec3, dist: f32) -> Vec3 {
    let y = vrefp(dist);
    let k = -(y * dist - 1.0);
    v * (y * k + y)
}

/// What F-11472 (undirected, the query's byte +212 = 0) leaves for the result list on a sphere or capsule bound: the
/// depth (+176 of its frame), the place along the query's capsule t (+180, the record's +64), the contact normal (+192)
/// and the record's point (the query's axis point less the normal (R - depth), back about the centroid). The part index
/// the material is asked for is 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CapsulePrimHit {
    pub depth: f32,
    pub t: f32,
    pub normal: Vec3,
    pub point: Vec3,
}

/// F-98480: the quick answer of the segment `a` + `d` u against the y axis over y0 .. y1 - (t, s) when it has one. A
/// segment all but upright (|d|^2 across under |0.01 d.y|): the middle of the overlap of the two spans (where they
/// overlap), t by division by d.y, s by y1 - y0 (0.5 when that is 1e-12 or less); else the segment's nearest approach
/// to the axis across (0.5 without length across), s from the height there (0.5 for a span of 1e-6 or less), an answer
/// only with both strictly inside (0, 1).
fn axis_quick(a: Vec3, d: Vec3, y0: f32, y1: f32) -> Option<(f32, f32)> {
    let dx2 = d.x * d.x;
    let across = fma(d.z, d.z, dx2);
    if !(across >= (d.y * 0.01).abs()) {
        let (lo, hi) = if d.y > 0.0 { (a.y, a.y + d.y) } else { (a.y + d.y, a.y) };
        let (top, bottom) = if y1 > y0 { (y1, y0) } else { (y0, y1) };
        if !(hi > bottom) || !(lo < top) {
            return None;
        }
        let lo = if lo >= bottom { lo } else { bottom };
        let hi = if hi > top { top } else { hi };
        let sum = lo + hi;
        let span = y1 - y0;
        let t = fms(sum, 0.5, a.y) / d.y;
        let s = if span.abs() > f32::from_bits(0x2B8C_BCCC) { fms(sum, 0.5, y0) / span } else { 0.5 };
        return Some((t, s));
    }
    let h2 = fma(d.z, d.z, dx2);
    let t = if h2 > f32::from_bits(0x2B8C_BCCC) { -(fma(a.z, d.z, a.x * d.x) / h2) } else { 0.5 };
    let span = y1 - y0;
    let s = if span.abs() > f32::from_bits(0x3586_37BD) { (fma(d.y, t, a.y) - y0) / span } else { 0.5 };
    (t > 0.0 && t < 1.0 && s > 0.0 && s < 1.0).then_some((t, s))
}

/// F-40164: the nearest places of the segment `a` + `d` t (t in [0, 1]) and the y axis over y0 .. y1 (s = 0 at y0, 1 at
/// y1): the quick answer (F-98480) where it has one, else by where each end's height falls (below, within, above the
/// span), comparing the line distances (F-43156) of the span's ends from the segment's line with the ends' distances
/// across from the axis, each end's projection clamped as F-76120 forms it. (t, s).
fn seg_axis(a: Vec3, d: Vec3, y0: f32, y1: f32) -> (f32, f32) {
    if let Some(ts) = axis_quick(a, d, y0, y1) {
        return ts;
    }
    let e = a + d;
    let span = y1 - y0;
    let inv = if span.abs() > f32::from_bits(0x2B8C_BCCC) { 1.0 / span } else { 0.0 };
    let param = |y: f32| if !(y > y0) { 0.0 } else if !(y < y1) { 1.0 } else { (y - y0) * inv };
    let (ta, tb) = (param(a.y), param(e.y));
    let p = |w: Vec3| clamp_param(0.0 - vdot(w, d), vdot(d, d));
    let at = |v: Vec3, y: f32| Vec3::new(v.x, v.y - y, v.z);
    let ld = |w: Vec3| line_dist(w, d);
    let across_b = || fma(e.x, e.x, e.z * e.z).sqrt();
    if !(ta > 0.0) {
        let w = at(a, y0);
        if !(tb > 0.0) {
            (p(w), 0.0)
        } else if tb < 1.0 {
            if ld(w) >= across_b() { (1.0, tb) } else { (p(w), 0.0) }
        } else {
            let w2 = at(a, y1);
            if ld(w) >= ld(w2) { (p(w2), 1.0) } else { (p(w), 0.0) }
        }
    } else if !(ta < 1.0) {
        let w2 = at(a, y1);
        if !(tb > 0.0) {
            let w = at(a, y0);
            if ld(w) >= ld(w2) { (p(w2), 1.0) } else { (p(w), 0.0) }
        } else if !(tb < 1.0) {
            (p(w2), 1.0)
        } else if ld(w2) >= across_b() {
            (1.0, tb)
        } else {
            (p(w2), 1.0)
        }
    } else if !(tb > 0.0) {
        let w = at(a, y0);
        if ld(w) >= fma(a.x, a.x, a.z * a.z).sqrt() { (0.0, ta) } else { (p(w), 0.0) }
    } else if tb == 1.0 {
        let w2 = at(a, y1);
        if ld(w2) >= fma(a.z, a.z, a.x * a.x).sqrt() { (0.0, ta) } else { (p(w2), 1.0) }
    } else {
        let (fb, fa) = (fma(e.x, e.x, e.z * e.z).sqrt(), fma(a.z, a.z, a.x * a.x).sqrt());
        if fa >= fb { (1.0, tb) } else { (0.0, ta) }
    }
}

/// F-11472, the undirected branch (the query's byte +212 = 0), on a sphere or capsule bound: the query's capsule `a` ..
/// `b` (its +176 / +192 in the bound's frame) of radius `big_r` (+208). The query's axis point nearest the bound (a
/// sphere: F-76120's clamped projection of the centroid; a capsule: F-40164 against its axis over -L/2 .. L/2), the
/// bound's point there (the centroid, or the axis at -L/2 + s L), and within 1e-12 a normal of its own: across the
/// query's axis for a sphere (F-95825's first direction across its unit direction), +x for a capsule (F-41334 with
/// 1e-5 on a vector under 1e-12: its fallback). A box (type 3) is `capsule_box`; None where the original leaves no
/// record and no list call (a box out of reach). Rules in docs/reference/people/traffic_drive_rule.md 9.7 / 9.8; checked to the bit against the original run offline
/// (`capsule_primitive_matches_the_original`).
pub fn capsule_primitive(a: Vec3, b: Vec3, big_r: f32, prim: &Primitive) -> Option<CapsulePrimHit> {
    let d = b - a;
    let (centre, reach, t, q) = match *prim {
        Primitive::Sphere { centre, r } => {
            let a0 = a - centre;
            (centre, r + big_r, clamp_param(0.0 - vdot(a0, d), vdot(d, d)), Vec3::ZERO)
        }
        Primitive::Capsule { centre, r, len } => {
            let h = len * 0.5;
            let (t, s) = seg_axis(a - centre, d, -h, h);
            (centre, r + big_r, t, Vec3::new(0.0, fnms(s, len, h), 0.0))
        }
        Primitive::Box { centre, size } => return capsule_box(a, b, big_r, centre, size),
    };
    let a0 = a - centre;
    let p = d * t + a0;
    let w = q + p;
    let d2 = fma(w.x, w.x, fma(w.z, w.z, w.y * w.y));
    let (depth, normal) = if d2 > f32::from_bits(0x2B8C_BCCC) {
        let dist = d2.sqrt();
        (reach - dist, unit_by(w, dist))
    } else {
        match *prim {
            Primitive::Sphere { .. } => (reach, super::solver::basis(d * rsqrt0(vdot(d, d)))[1]),
            _ => (reach, Vec3::X),
        }
    };
    // (the negation kept apart from the sum: folded into c - x, a NaN of a capsule without length would keep the sign the
    // vector unit's negation flips)
    let point = std::hint::black_box(-(normal * (big_r - depth) - p)) + centre;
    Some(CapsulePrimHit { depth, t, normal, point })
}

/// Each box edge's two corners (the image's table, indices into `BOX_CORNER`); the index -1 reads the word before it:
/// 7 and 11.
const EDGE_CORNERS: [(usize, usize); 12] = [(0, 1), (1, 2), (2, 3), (3, 0), (4, 5), (5, 6), (6, 7), (7, 4), (0, 4), (1, 5), (2, 6), (3, 7)];

/// F-21753: the face of the box (half sizes `half` about 0) nearest the point `p` from inside - the least of h - p and
/// p + h per axis (+x -x +y -y +z -z), each strictly under the least before; -1 when none is under FLT_MAX.
fn near_face(half: Vec3, p: Vec3) -> i32 {
    let mut best = f32::MAX;
    let mut face = -1;
    for ax in 0..3 {
        let a = half[ax] - p[ax];
        if a < best {
            best = a;
            face = 2 * ax as i32;
        }
        let b = p[ax] + half[ax];
        if b < best {
            best = b;
            face = 2 * ax as i32 + 1;
        }
    }
    face
}

/// F-94282: the edge between the faces `fa` and `fb` (its index in the faces' table `BOX_EDGE`, either order), -1
/// for none.
fn edge_between(fa: i32, fb: i32) -> i32 {
    BOX_EDGE
        .iter()
        .position(|&(a, b)| (a as i32 == fa && b as i32 == fb) || (a as i32 == fb && b as i32 == fa))
        .map_or(-1, |k| k as i32)
}

/// F-79106, undirected (the query's byte +212 = 0): the piece `start` .. `end` (direction `dir`) of the capsule's axis
/// cut where it crosses the box's face plane `code` (+x -x +y -y +z -z) at `t` of the piece. The crossing replaces the
/// end when the start lies nearer the box along the region's axis (outside both other slabs: by the two others'
/// squares), else the start; `dir` becomes end - start. Then the feature of the piece's middle (F-25936); for a middle
/// inside the box the faces nearest the two ends (F-21753): the same face; opposite faces - the least of the four other
/// faces' pushes (h - min, max + h); neighbours - their edge when the piece is longer than the difference of the ends'
/// squared distances from that edge's line, else the face with the larger of the two least depths. (kind, index).
fn box_clip(code: i32, half: Vec3, start: &mut Vec3, end: &mut Vec3, dir: &mut Vec3, t: f32) -> (i32, i32) {
    let c = *dir * t + *start;
    let axis = (code / 2) as usize;
    let j1 = (axis + 1) % 3;
    let j2 = (j1 + 1) % 3;
    let ac = c.abs();
    let (s, e) = (*start, *end);
    let nearer_start = if !(ac[j1] < half[j1]) {
        if ac[j2] < half[j2] {
            s[j1].abs() < e[j1].abs()
        } else {
            fma(s[j2], s[j2], s[j1] * s[j1]) < fma(e[j2], e[j2], e[j1] * e[j1])
        }
    } else if ac[j2] < half[j2] {
        s[axis].abs() < e[axis].abs()
    } else {
        s[j2].abs() < e[j2].abs()
    };
    if nearer_start {
        *end = c;
    } else {
        *start = c;
    }
    let (s, e) = (*start, *end);
    *dir = e - s;
    let (kind, index) = box_feature((e + s) * 0.5, half);
    if kind != -1 {
        return (kind, index as i32);
    }
    let fa = near_face(half, s);
    let fb = near_face(half, e);
    if fa == fb {
        #[cfg(test)]
        tests::BOX_WAYS.with(|w| w.borrow_mut()[0] += 1);
        return (2, fa);
    }
    if (fb == fa + 1 && fa % 2 == 0) || (fa == fb + 1 && fb % 2 == 0) {
        let mut best = f32::MAX;
        let mut face = 0;
        for ax in 0..3 {
            let (hi, lo) = if s[ax] > e[ax] { (s[ax], e[ax]) } else { (e[ax], s[ax]) };
            let (f0, f1) = (2 * ax as i32, 2 * ax as i32 + 1);
            if fa != f0 && fb != f0 {
                let v = half[ax] - lo;
                if v < best {
                    best = v;
                    face = f0;
                }
            }
            if fa != f1 && fb != f1 {
                let v = hi + half[ax];
                if v < best {
                    best = v;
                    face = f1;
                }
            }
        }
        #[cfg(test)]
        tests::BOX_WAYS.with(|w| w.borrow_mut()[1] += 1);
        return (2, face);
    }
    let (dx, dy, dz) = (s.x - e.x, s.y - e.y, s.z - e.z);
    let (gs1, ge1) = (half[j1] - s[j1].abs(), half[j1] - e[j1].abs());
    let (gs2, ge2) = (half[j2] - s[j2].abs(), half[j2] - e[j2].abs());
    let qs = fma(gs2, gs2, gs1 * gs1);
    let qe = fma(ge2, ge2, ge1 * ge1);
    let len = fma(dx, dx, fma(dz, dz, dy * dy)).sqrt();
    #[cfg(test)]
    tests::BOX_WAYS.with(|w| w.borrow_mut()[if len > (qs - qe).abs() { 2 } else { 3 }] += 1);
    if len > (qs - qe).abs() {
        return (1, edge_between(fa, fb));
    }
    let (a, b) = ((fa / 2) as usize, (fb / 2) as usize);
    let least_b = if s[b].abs() - e[b].abs() >= 0.0 { e[b].abs() } else { s[b].abs() };
    let least_a = if s[a].abs() - e[a].abs() >= 0.0 { e[a].abs() } else { s[a].abs() };
    if half[a] - least_a < half[b] - least_b { (2, fa) } else { (2, fb) }
}

/// F-11472, the undirected branch, on a box bound (type 3: size +208, about the centroid +48): the query's capsule
/// `a` .. `b` of radius `big_r`. The axis's crossings of the six face planes within [0, 1]; none: the feature of its
/// middle (F-25936); one: the axis cut there (F-79106), the kept piece's range by whether its start stayed within 1e-6
/// of the axis's start; more: the crossing whose point lies least outside its face (the distance in the face's plane,
/// inside the larger gap), the piece between its neighbouring crossings cut there. By the feature: a corner (F-36467 on
/// the piece, within R; the normal from the corner, within 1e-12 the corner's +-1 x 0.57735), an edge (F-45690 of the
/// edge against the piece, the stack word holding 2; within 1e-12 the unit of axis x edge facing the point; t by the
/// distance from the axis's start over its length), a face (the axis's end nearer it; the chosen crossing for a face of
/// another axis), or the whole axis inside (the least push out through a face, t 0); a point inside the box adds the
/// distance to R. None where the original makes no record (out of reach). Rules in
/// docs/reference/people/traffic_drive_rule.md 9.8; checked to the bit against the original run offline
/// (`capsule_primitive_matches_the_original`).
fn capsule_box(a: Vec3, b: Vec3, big_r: f32, centre: Vec3, size: Vec3) -> Option<CapsulePrimHit> {
    let eps = f32::from_bits(0x2B8C_BCCC);
    let d = b - a;
    let a0 = a - centre;
    let half = size * 0.5;
    let dd = fma(d.x, d.x, fma(d.y, d.y, d.z * d.z));
    let dlen = if dd > 0.0 { dd.sqrt() } else { 0.0 };
    // the frame's word +216: each axis's second crossing, then the cut's place; what it holds before (when no axis
    // has a direction) is the caller's, read only for an edge under 1e-6
    let mut s216 = 0u32;
    // the face planes' crossings: t and the face's code
    let mut ts: Vec<f32> = Vec::with_capacity(6);
    let mut codes: Vec<i32> = Vec::with_capacity(6);
    for ax in 0..3 {
        if d[ax] == 0.0 {
            continue;
        }
        let inv = 1.0 / d[ax];
        for (k, plane) in [(0, half[ax]), (1, -half[ax])] {
            let t = (plane - a0[ax]) * inv;
            if k == 1 {
                s216 = t.to_bits();
            }
            if !(t < 0.0) && !(t > 1.0) {
                ts.push(t);
                codes.push(2 * ax as i32 + k);
            }
        }
    }
    let n = ts.len();
    let mut start = a0;
    let mut end = a0 + d;
    let mut dir = d;
    let (mut t_lo, mut t_hi) = (0.0f32, 1.0f32);
    let mut chosen = 0usize;
    let (kind, index) = if n == 0 {
        let (k, i) = box_feature(d * 0.5 + a0, half);
        (k, i as i32)
    } else if n == 1 {
        let ki = box_clip(codes[0], half, &mut start, &mut end, &mut dir, ts[0]);
        let tol = f32::from_bits(0x3586_37BD);
        let kept = (0..3).all(|k| a0[k] + tol >= start[k] && start[k] >= a0[k] - tol);
        #[cfg(test)]
        tests::BOX_WAYS.with(|w| w.borrow_mut()[if kept { 4 } else { 5 }] += 1);
        if kept {
            t_hi = ts[0];
        } else {
            t_lo = ts[0];
        }
        ki
    } else {
        // how far outside its face each crossing lies
        let mut out = [0.0f32; 6];
        for i in 0..n {
            let p = (d * ts[i] + a0).abs();
            let j1 = ((codes[i] / 2 + 1) % 3) as usize;
            let j2 = (j1 + 1) % 3;
            let (h1, p1, p2, h2) = (half[j1], p[j1], p[j2], half[j2]);
            out[i] = if p1 > h1 {
                if p2 > h2 {
                    let (u, v) = (p1 - h1, p2 - h2);
                    fma(v, v, u * u).sqrt()
                } else {
                    p1 - h1
                }
            } else if p2 > h2 {
                p2 - h2
            } else {
                let (g1, g2) = (p1 - h1, p2 - h2);
                if !(g2.abs() < g1.abs()) { g1 } else { g2 }
            };
        }
        for k in 1..n {
            if out[k] < out[chosen] {
                chosen = k;
            }
        }
        let tr = ts[chosen];
        for (k, &tk) in ts.iter().enumerate() {
            if k == chosen {
                continue;
            }
            if tk < tr {
                if tk > t_lo {
                    t_lo = tk;
                }
            } else if tk > tr && tk < t_hi {
                t_hi = tk;
            }
        }
        let rel = (tr - t_lo) / (t_hi - t_lo);
        s216 = rel.to_bits();
        start = d * t_lo + a0;
        end = d * t_hi + a0;
        dir = end - start;
        box_clip(codes[chosen], half, &mut start, &mut end, &mut dir, rel)
    };
    #[cfg(test)]
    tests::LAST_BOX.with(|c| c.set((kind, n)));
    let inside = |p: Vec3| {
        !(p.x > half.x) && !(p.x < -half.x) && !(p.y > half.y) && !(p.y < -half.y) && !(p.z > half.z) && !(p.z < -half.z)
    };
    let (depth, t, normal) = match kind {
        0 => {
            let corner = (Vec3::from(BOX_CORNER[index as usize]) * half) * 2.0;
            let tc = clamp_param(0.0 - vdot(start - corner, dir), vdot(dir, dir));
            let sp = dir * tc + start;
            let w = sp - corner;
            let w2 = fma(w.x, w.x, fma(w.z, w.z, w.y * w.y));
            if w2 > big_r * big_r {
                return None;
            }
            let t = fma(t_hi - t_lo, tc, t_lo);
            if w2 > eps {
                let dist = w2.sqrt();
                (if inside(sp) { big_r + dist } else { big_r - dist }, t, unit_by(w, dist))
            } else {
                #[cfg(test)]
                tests::BOX_WAYS.with(|w| w.borrow_mut()[6] += 1);
                (big_r, t, (Vec3::from(BOX_CORNER[index as usize]) * 2.0) * f32::from_bits(0x3F13_CD3A))
            }
        }
        1 => {
            let (c0, c1) = if index < 0 { (7, 11) } else { EDGE_CORNERS[index as usize] };
            let k = half * 2.0;
            let p = Vec3::from(BOX_CORNER[c0]) * k;
            let e = Vec3::from(BOX_CORNER[c1]) * k - p;
            // (F-45690 with the piece first: the piece's parameter goes to the frame's word +216, which keeps what
            // the crossings left there when the edge is shorter than 1e-6, the edge's to +184)
            let mut slot = s216;
            let (_, _, te) = segment_params(start, dir, p, e, &mut slot);
            let ep = e * te + p;
            let sp = dir * f32::from_bits(slot) + start;
            let mut w = (Vec3::ZERO - ep) + sp;
            let (fx, fy, fz) = (sp.x - a0.x, sp.y - a0.y, sp.z - a0.z);
            let t = fma(fx, fx, fma(fz, fz, fy * fy)).sqrt() / dlen;
            let w2 = fma(w.x, w.x, fma(w.z, w.z, w.y * w.y));
            if w2 > eps {
                let within = inside(sp);
                if within {
                    w = Vec3::ZERO - w;
                } else if w2 > big_r * big_r {
                    return None;
                }
                let dist = w2.sqrt();
                (if within { big_r + dist } else { big_r - dist }, t, unit_by(w, dist))
            } else {
                #[cfg(test)]
                tests::BOX_WAYS.with(|w| w.borrow_mut()[7] += 1);
                let c = vcross(d, e);
                let mut nrm = c * rsqrt0(vdot(c, c));
                if fma(sp.x, nrm.x, fma(sp.z, nrm.z, sp.y * nrm.y)) < 0.0 {
                    nrm = Vec3::ZERO - nrm;
                }
                (big_r, t, nrm)
            }
        }
        2 => {
            let ax = (index / 2) as usize;
            let (mut t, mut base) = (0.0f32, a0);
            if !(start[ax].abs() < end[ax].abs()) {
                t = 1.0;
                base = a0 + d;
            }
            let nrm = BOX_FACE[index as usize];
            let q = -(nrm * big_r - base);
            let depth = if nrm[ax] * q[ax] >= 0.0 { half[ax] - q[ax].abs() } else { q[ax].abs() + half[ax] };
            if depth < 0.0 {
                return None;
            }
            if n > 0 && codes[chosen] / 2 != ax as i32 {
                #[cfg(test)]
                tests::BOX_WAYS.with(|w| w.borrow_mut()[8] += 1);
                t = ts[chosen];
                let p = d * t + a0;
                let dn = fma(p.x, nrm.x, fma(p.z, nrm.z, p.y * nrm.y));
                ((half[ax] - dn) + big_r, t, nrm)
            } else {
                (depth, t, nrm)
            }
        }
        _ => {
            // the whole axis inside: the least push out through a face
            let mut best = f32::MAX;
            let mut face: Option<(usize, f32)> = None;
            for ax in 0..3 {
                let mut g = half[ax] - a0[ax];
                if d[ax] < 0.0 {
                    g = g - d[ax];
                }
                if g < best {
                    best = g;
                    face = Some((ax, 1.0));
                }
                let mut g = a0[ax] + half[ax];
                if d[ax] > 0.0 {
                    g = d[ax] + g;
                }
                if g < best {
                    best = g;
                    face = Some((ax, -1.0));
                }
            }
            let mut nrm = Vec3::ZERO;
            if let Some((ax, s)) = face {
                nrm[ax] = s;
            }
            (big_r + best, 0.0, nrm)
        }
    };
    let p = d * t + a0;
    let point = std::hint::black_box(-(normal * (big_r - depth) - p)) + centre;
    Some(CapsulePrimHit { depth, t, normal, point })
}

/// F-15615 on a sphere, capsule or box bound, in the bound's frame: the ball at `c` (the query's +160) of radius `big_r`
/// (+176). Rules in docs/reference/people/traffic_drive_rule.md 9.7. The record's point is
/// -(normal (R - depth) - c); a depth above 0 takes a slot. Checked to the bit against the original run offline
/// (`ball_primitive_matches_the_original`).
pub fn ball_primitive(c: Vec3, big_r: f32, b: &Primitive) -> PrimContact {
    let eps = f32::from_bits(0x2B8C_BCCC);
    let sq = |w: Vec3| fma(w.y, w.y, fma(w.x, w.x, w.z * w.z));
    match *b {
        Primitive::Sphere { centre, r } => {
            let d = c - centre;
            let d2 = sq(d);
            let (dist, normal) = if d2 > eps { (d2.sqrt(), unit_by(d, d2.sqrt())) } else { (0.0, Vec3::X) };
            PrimContact { depth: (big_r + r) - dist, normal, part: 0, kind: 2 }
        }
        Primitive::Capsule { centre, r, len } => {
            let d = c - centre;
            let h = len * 0.5;
            let lo = if d.y - -h >= 0.0 { d.y } else { -h };
            let yc = if lo - h >= 0.0 { h } else { lo };
            let dy = d.y - yc;
            let d2 = fma(d.z, d.z, fma(dy, dy, d.x * d.x));
            let reach = r + big_r;
            if d2 > reach * reach {
                return PrimContact { depth: -1.0, normal: d, part: 0, kind: 2 };
            }
            let w = Vec3::new(d.x, dy, d.z);
            let (dist, normal) = if d2 > eps { (d2.sqrt(), unit_by(w, d2.sqrt())) } else { (0.0, Vec3::X) };
            let part = if !(d.y < h) { 1 } else if d.y > -h { 2 } else { 0 };
            PrimContact { depth: reach - dist, normal, part, kind: 2 }
        }
        Primitive::Box { centre, size } => {
            let d = c - centre;
            let half = size * 0.5;
            let (feature, id) = box_feature(d, half);
            let corner = |k: usize| Vec3::from(BOX_CORNER[k]);
            match feature {
                0 => {
                    let w = d - (corner(id) * 2.0) * half;
                    let d2 = sq(w);
                    if d2 > eps {
                        let dist = d2.sqrt();
                        PrimContact { depth: big_r - dist, normal: unit_by(w, dist), part: id as u16, kind: 0 }
                    } else {
                        let v = corner(id);
                        PrimContact { depth: big_r, normal: v * rsqrt0(vdot(v, v)), part: id as u16, kind: 0 }
                    }
                }
                1 => {
                    let (fa, fb) = BOX_EDGE[id];
                    let (a, bb) = (fa / 2, fb / 2);
                    let third = 3 - bb - a;
                    let mut p = d;
                    p[a] = if d[a] > 0.0 { half[a] } else { -half[a] };
                    p[bb] = if d[bb] > 0.0 { half[bb] } else { -half[bb] };
                    let w = d - p;
                    let d2 = sq(w);
                    let (depth, normal) = if d2 > eps {
                        let dist = d2.sqrt();
                        (big_r - dist, unit_by(w, dist))
                    } else {
                        let mut v = corner(id);
                        v[third] = 0.0;
                        (big_r, v * rsqrt0(vdot(v, v)))
                    };
                    let (na, nb) = (BOX_FACE[fa], BOX_FACE[fb]);
                    let da = fma(normal.x, na.x, fma(na.z, normal.z, na.y * normal.y));
                    let db = fma(normal.x, nb.x, fma(nb.z, normal.z, nb.y * normal.y));
                    PrimContact { depth, normal, part: if da > db { fa as u16 } else { fb as u16 }, kind: 2 }
                }
                2 => {
                    let ax = id / 2;
                    PrimContact { depth: (half[ax] - d[ax].abs()) + big_r, normal: BOX_FACE[id], part: id as u16, kind: 2 }
                }
                _ => {
                    let g = half - d.abs();
                    let ax = if g.x > g.y { if g.y > g.z { 2 } else { 1 } } else if g.x > g.z { 2 } else { 0 };
                    let face = if d[ax] > 0.0 { 2 * ax } else { 2 * ax + 1 };
                    PrimContact { depth: (half[ax] - d[ax].abs()) + big_r, normal: BOX_FACE[face], part: face as u16, kind: 2 }
                }
            }
        }
    }
}

/// The 42 directions F-96503 tries (the image's table, as `gjk` holds it for the vehicles' routine).
const WAYS_OUT: [[u32; 3]; 42] = [
    [0x00000000, 0x80000000, 0xbf800000], [0x3f393e60, 0xbf0695ea, 0xbee4f9e4], [0xbe8d82ba, 0xbf59c422, 0xbee4f9e4],
    [0xbf64f91a, 0x80000000, 0xbee4f97f], [0xbe8d82ba, 0x3f59c422, 0xbee4fa05], [0x3f393e60, 0x3f0695ea, 0xbee4f9e4],
    [0x3e8d82ba, 0xbf59c422, 0x3ee4fa05], [0xbf393e60, 0xbf0695ea, 0x3ee4f9e4], [0xbf393e60, 0x3f0695ea, 0x3ee4f9e4],
    [0x3e8d82ba, 0x3f59c422, 0x3ee4f9e4], [0x3f64f91a, 0x00000000, 0x3ee4f97f], [0x80000000, 0x00000000, 0x3f800000],
    [0x3ed9c3f0, 0xbe9e36b1, 0xbf59c476], [0xbe265ade, 0xbeffff58, 0xbf59c476], [0x3e8696c4, 0xbf4f1b69, 0xbf0696c4],
    [0x3ed9c3f0, 0x3e9e36b1, 0xbf59c476], [0x3f59c411, 0x80000000, 0xbf0696a2], [0xbf06963e, 0x80000000, 0xbf59c454],
    [0xbf302d38, 0xbeffff9b, 0xbf0696a2], [0xbe265ade, 0x3effff58, 0xbf59c476], [0xbf302d38, 0x3effff9b, 0xbf0696a2],
    [0x3e8696c4, 0x3f4f1b69, 0xbf0696c4], [0x3f737889, 0x3e9e36f4, 0x00000000], [0x3f737889, 0xbe9e36f4, 0x00000000],
    [0x3f167925, 0xbf4f1bbd, 0x00000000], [0x00000000, 0xbf800000, 0x00000000], [0xbf167925, 0xbf4f1bbd, 0x00000000],
    [0xbf737889, 0xbe9e36f4, 0x80000000], [0xbf737889, 0x3e9e36f4, 0x80000000], [0xbf167925, 0x3f4f1bbd, 0x80000000],
    [0x80000000, 0x3f800000, 0x80000000], [0x3f167925, 0x3f4f1bbd, 0x80000000], [0x3f302d38, 0xbeffff9b, 0x3f0696a2],
    [0xbe8696c4, 0xbf4f1b69, 0x3f0696c4], [0xbf59c411, 0x00000000, 0x3f0696a2], [0xbe8696c4, 0x3f4f1b69, 0x3f0696c4],
    [0x3f302d38, 0x3effff9b, 0x3f0696a2], [0x3f06963e, 0x00000000, 0x3f59c454], [0x3e265ade, 0xbeffff58, 0x3f59c476],
    [0xbed9c3f0, 0xbe9e36b1, 0x3f59c476], [0xbed9c3f0, 0x3e9e36b1, 0x3f59c476], [0x3e265ade, 0x3effff58, 0x3f59c476],
];

/// F-96503, the convex-against-convex penetration routine, on the shapes `a` (margin `ma`) and `b` (`mb`), each at its
/// own matrix: of the 42 directions d the one with the least d . (b's farthest along d - a's farthest against d) (strict
/// <, from FLT_MAX); off = that + ((ma + mb) + 0.01), 0 where below 0; the detector (F-15933: first direction (0, 0, 1),
/// no penetration routine of its own) and its loop F-33272 with `a` moved off along it; where the loop answers, the two
/// points (on b's surface less the normal (off (d . n) - distance), and on b's surface).
pub fn penetration(a: &impl gjk::Support, ma: f32, b: &impl gjk::Support, mb: f32) -> Option<(Vec3, Vec3)> {
    let mut best = (f32::MAX, Vec3::ZERO);
    for w in WAYS_OUT {
        let d = Vec3::new(f32::from_bits(w[0]), f32::from_bits(w[1]), f32::from_bits(w[2]));
        let (pa, pb) = (a.support(Vec3::ZERO - d), b.support(d));
        let delta = vdot(d, pb - pa);
        if best.0 > delta {
            best = (delta, d);
        }
    }
    let mut off = best.0 + ((ma + mb) + f32::from_bits(0x3C23_D70A));
    if 0.0 > off {
        off = 0.0;
    }
    let mut v = Vec3::Z;
    let (inner, _) = gjk::loop_from(a, best.1 * off, Vec3::ZERO, b, &mut v, ma, mb);
    inner.map(|r| {
        let pen = off * vdot(best.1, r.normal) - r.distance;
        (r.on_b - r.normal * pen, r.on_b)
    })
}

/// F-15615 on a curved bound (case 5: a car's wheel, `wheel` at the bound's own frame, its centroid +48 and margin
/// +112), in the bound's frame: the ball at `c` (the query's +160) of radius `big_r` (+176) made a sphere bound
/// (F-80030) at `c`, and F-96503 between the two. Where it answers: n = (its point a - its point b); n . (c - centroid)
/// < 0 gives depth -1 and that n; else |n| (fused z, x, y) when over 1e-12 with n by the estimate and one Newton step,
/// otherwise depth 0 and (c - centroid)'s unit (the guarded rsqrt). No answer: depth -1, the normal c - centroid. The
/// record as for the other primitives (part 0, kind 2). Checked to the bit against the original run offline
/// (`ball_curved_matches_the_original`).
pub fn ball_curved(c: Vec3, big_r: f32, wheel: &gjk::WheelCore, centroid: Vec3, margin: f32) -> PrimContact {
    let ball = gjk::PartCore { kind: 0, centroid: Vec3::ZERO, half: Vec3::ZERO, rows: [Vec3::X, Vec3::Y, Vec3::Z], at: c };
    let off = c - centroid;
    let (depth, normal) = match penetration(wheel, margin, &ball, big_r) {
        Some((pa, pb)) => {
            let n = pa - pb;
            if fma(n.x, off.x, fma(n.z, off.z, n.y * off.y)) < 0.0 {
                (-1.0, n)
            } else {
                let l2 = fma(n.y, n.y, fma(n.x, n.x, n.z * n.z));
                if l2 > f32::from_bits(0x2B8C_BCCC) {
                    let dist = l2.sqrt();
                    (dist, unit_by(n, dist))
                } else {
                    (0.0, off * rsqrt0(vdot(off, off)))
                }
            }
        }
        None => (-1.0, off),
    };
    PrimContact { depth, normal, part: 0, kind: 2 }
}

/// F-11472 undirected (the query's byte +212 = 0) on a curved bound (type 5: a car's wheel, `wheel` at the bound's own
/// frame, its centroid +48 and margin +112): the query's capsule `a` .. `b` (radius `big_r`) made a capsule bound
/// (F-25220 with F-77704: radius R, length L = |b - a| (fused y, z, x, no guard), along y of the frame [F-95825's third
/// axis, the unit direction (by the estimate when L > 1e-12, else +x), its second axis] about (b + a) 0.5), and F-96503
/// between the wheel and it. Where it answers: t = F-36467 (the segment from a - centroid along b - a, to its point b),
/// n = its point a - its point b; n . ((b0 + a0) 0.5) < 0 leaves depth 0 and that n; else |n| (fused y, z, x) over 1e-12
/// with n by the estimate and one Newton step, otherwise depth 0 and the unit (guarded rsqrt) of (b - a) t + a0 - point b.
/// No answer: depth 0, t 0, the normal 0. The record's point -(n (R - depth) - p) + centroid, p = (b - a) t + a0.
/// Checked to the bit against the original run offline (`capsule_curved_matches_the_original`).
pub fn capsule_curved(a: Vec3, b: Vec3, big_r: f32, wheel: &gjk::WheelCore, centroid: Vec3, margin: f32) -> CapsulePrimHit {
    let d = b - a;
    let (a0, b0) = (a - centroid, b - centroid);
    let len = fma(d.x, d.x, fma(d.y, d.y, d.z * d.z)).sqrt();
    let dir = if len > f32::from_bits(0x2B8C_BCCC) { unit_by(d, len) } else { Vec3::X };
    let [_, across, third] = super::solver::basis(dir);
    let mid = (b + a) * 0.5;
    let caps = gjk::PartCore { kind: 1, centroid: Vec3::ZERO, half: Vec3::splat(len), rows: [third, dir, across], at: mid };
    let (depth, t, normal) = match penetration(wheel, margin, &caps, big_r) {
        Some((pa, pb)) => {
            let t = clamp_param(0.0 - vdot(a0 - pb, d), vdot(d, d));
            let m = (b0 + a0) * 0.5;
            let n = pa - pb;
            if fma(n.y, m.y, fma(n.x, m.x, n.z * m.z)) < 0.0 {
                (0.0, t, n)
            } else {
                let l2 = fma(n.x, n.x, fma(n.z, n.z, n.y * n.y));
                if l2 > f32::from_bits(0x2B8C_BCCC) {
                    let dist = l2.sqrt();
                    (dist, t, unit_by(n, dist))
                } else {
                    let w = (d * t + a0) - pb;
                    (0.0, t, w * rsqrt0(vdot(w, w)))
                }
            }
        }
        None => (0.0, 0.0, Vec3::ZERO),
    };
    let p = d * t + a0;
    let point = std::hint::black_box(-(normal * (big_r - depth) - p)) + centroid;
    CapsulePrimHit { depth, t, normal, point }
}

/// Whether the capsule `a` .. `b` (radius `r`, in the part's frame) misses the part bound's box `min` .. `max`
/// (F-16177 -> F-81407 -> F-18932): the box's centre taken off both ends and `r` added to its half sizes, then the
/// segment against the box grown by 1e-6 by separating axes - the three box axes (|mid| over half + |half segment|),
/// then the three crosses of a box axis with the segment (|mid x half segment| over the box's projection, fused in
/// the original's orders). A NaN separates nowhere.
pub fn part_dropped(a: Vec3, b: Vec3, r: f32, max: Vec3, min: Vec3) -> bool {
    let centre = (max + min) * 0.5;
    let half = (max + (Vec3::ZERO - min)) * 0.5 + Vec3::splat(r);
    let (a, b) = (a - centre, b - centre);
    let h = half + Vec3::splat(f32::from_bits(0x3586_37BD));
    let mid = (b + a) * 0.5;
    let seg = (b + (Vec3::ZERO - a)) * 0.5;
    let (m, w) = (mid.abs(), seg.abs());
    let c = (yzx(mid) * zxy(seg) - zxy(mid) * yzx(seg)).abs();
    let axes = m.x > w.x + h.x || m.y > w.y + h.y || m.z > w.z + h.z;
    let cx = c.x > fma(h.z, w.y, w.z * h.y);
    let cyz = c.y > fma(w.z, h.x, h.z * w.x) || c.z > fma(w.y, h.x, h.y * w.x);
    axes || cx || cyz
}

// ---- The level query's result list and the capsule's dispatch F-35029 (docs/reference/people/traffic_drive_rule.md
// 9.10). The list lives in the query: +0 / +16 the query's box, +32 the records (96 bytes each), +36 their number, +40
// a scratch record (0 for every query the sweep makes: the reset F-16848 clears it), +44 the records of the objects
// done, +48 those of this composite level, +52 the new ones (since the last commit), +64 / +68 the new records' bits,
// +72 / +76 the level's, +80 / +84 the highest / lowest score held.

/// One record of the result list as the narrow phases fill it: +0 the instance, +16 the contact, +32 its normal, +48
/// the bound's normal, +64 the place along the capsule, +68 the depth, +72 the material, +76 the polygon / part index,
/// +78 the kind, +80 the byte below the radius, +82 the composite's part (F-47011).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Record {
    pub instance: u32,
    pub point: Vec3,
    pub normal: Vec3,
    pub bound_normal: Vec3,
    pub t: f32,
    pub depth: f32,
    pub material: u32,
    pub index: u16,
    pub kind: u16,
    pub below: u8,
    pub part: u16,
}

impl Record {
    /// As the reset (F-16848 via the query's set-up) leaves a record: all 0 but the index 0xFFFF.
    pub const RESET: Record = Record {
        instance: 0,
        point: Vec3::ZERO,
        normal: Vec3::ZERO,
        bound_normal: Vec3::ZERO,
        t: 0.0,
        depth: 0.0,
        material: 0,
        index: 0xFFFF,
        kind: 0,
        below: 0,
        part: 0,
    };
}

/// The result list of a level query (see above). `records.len()` is +36 (0: the list only notes that something
/// touched, +52 = 1).
#[derive(Clone, Debug, PartialEq)]
pub struct ResultList {
    pub min: Vec3,
    pub max: Vec3,
    pub records: Vec<Record>,
    pub done: i32,
    pub level: i32,
    pub new: i32,
    pub new_bits: u64,
    pub level_bits: u64,
    pub high: f32,
    pub low: f32,
}

impl ResultList {
    /// The reset the query's set-up runs: `cap` records reset, the box -FLT_MAX .. FLT_MAX, the counts and bits 0, the
    /// scores -FLT_MAX / FLT_MAX.
    pub fn new(cap: usize) -> ResultList {
        assert!(cap <= 64, "the bits are two words");
        ResultList {
            min: Vec3::splat(-f32::MAX),
            max: Vec3::splat(f32::MAX),
            records: vec![Record::RESET; cap],
            done: 0,
            level: 0,
            new: 0,
            new_bits: 0,
            level_bits: 0,
            high: -f32::MAX,
            low: f32::MAX,
        }
    }

    /// F-89028: the slot for a contact of `depth` (the query's cap `cap`) at `at`, or none. Without records a depth above
    /// 0 sets +52 = 1. Else a depth above 0 whose point is not below the box on some axis and not above it on all three;
    /// its score the depth, or cap - depth when the depth is above the cap. A free slot (+48 + +44 + +52 under the
    /// number) is taken and the scores held widened; a full list replaces its lowest score (the first of them, the
    /// held records' depths scored the same way) when the new score is above +84, the slot made new (out of the level's
    /// bits, or of the objects done), +80 kept and +84 the second lowest of the scan (from +80) - with one record both
    /// the new score.
    pub fn alloc(&mut self, depth: f32, cap: f32, at: Vec3) -> Option<usize> {
        let n_cap = self.records.len();
        if n_cap == 0 {
            if depth > 0.0 {
                self.new = 1;
            }
            return None;
        }
        if !(depth > 0.0) {
            return None;
        }
        // (vcmpgefp / vcmpgtfp on (p, 1) against (box, 0): every lane, the w lane always true)
        if !(at.x >= self.min.x && at.y >= self.min.y && at.z >= self.min.z) {
            return None;
        }
        if at.x > self.max.x && at.y > self.max.y && at.z > self.max.z {
            return None;
        }
        let score = if depth > cap { cap - depth } else { depth };
        let n = self.level + self.done + self.new;
        if n < n_cap as i32 {
            self.new_bits |= 1 << n;
            self.high = if score - self.high >= 0.0 { score } else { self.high };
            self.new += 1;
            self.low = if score - self.low >= 0.0 { self.low } else { score };
            return Some(n as usize);
        }
        if !(score > self.low) {
            return None;
        }
        let (mut second, mut k, mut lowest) = (self.high, 0usize, f32::MAX);
        for (j, r) in self.records.iter().enumerate() {
            let d = if r.depth > cap { cap - r.depth } else { r.depth };
            if d < lowest {
                second = lowest;
                k = j;
                lowest = d;
            } else if d < second {
                second = d;
            }
        }
        let bit = 1u64 << k;
        if self.new_bits & bit == 0 {
            self.new_bits |= bit;
            self.new += 1;
            if self.level_bits & bit != 0 {
                self.level_bits &= !bit;
                self.level -= 1;
            } else {
                self.done -= 1;
            }
        }
        if n_cap <= 1 {
            self.high = score;
        }
        self.low = if n_cap > 1 { second } else { score };
        Some(k)
    }

    /// F-47011: the new records named as the composite's part `i` (+82), while there are new ones.
    pub fn mark_part(&mut self, i: u16) {
        if self.new <= 0 {
            return;
        }
        for (k, r) in self.records.iter_mut().enumerate() {
            if self.new_bits & (1 << k) != 0 {
                r.part = i;
            }
        }
    }

    /// F-77105: the new records carried out of the frame of rows `m[0..3]` and place `m[3]` (F-57315: the contact as
    /// (x, y, z, 1) by the columns, the vector unit's four-product sum; the normals by the vector unit's three-product
    /// sum), then counted - into this composite level (`level`: +48 = min(max(n, 1) - +44, +48 + +52)) or as done (+44 =
    /// min(max(n, 1), +52 + +44)); +52 and the new bits 0. Answers whether the list has no records (the dispatch's loop
    /// ends then).
    pub fn commit(&mut self, m: &[Vec3; 4], level: bool) -> bool {
        let col = |k: usize| Vec3::new(m[0][k], m[1][k], m[2][k]);
        let (cx, cy, cz) = (col(0), col(1), col(2));
        for (k, r) in self.records.iter_mut().enumerate() {
            if self.new_bits & (1 << k) == 0 {
                continue;
            }
            r.point = out_of(r.point, m);
            r.normal = Vec3::new(vdot(r.normal, cx), vdot(r.normal, cy), vdot(r.normal, cz));
            r.bound_normal = Vec3::new(vdot(r.bound_normal, cx), vdot(r.bound_normal, cy), vdot(r.bound_normal, cz));
        }
        let most = (self.records.len() as i32).max(1);
        if level {
            self.level = (most - self.done).min(self.level + self.new);
        } else {
            self.done = most.min(self.new + self.done);
        }
        self.new = 0;
        self.new_bits = 0;
        self.records.is_empty()
    }

    /// F-67153: the composite level's records become the new ones of the level above.
    pub fn close_level(&mut self) {
        self.new = self.level;
        self.level = 0;
        self.new_bits = self.level_bits;
        self.level_bits = 0;
    }

    /// F-37493: whether new records were added.
    pub fn any_new(&self) -> bool {
        self.new > 0
    }
}

/// One polygon of a polygon bound: its record's area (+0), its three vertices (+4) and the polygons across its edges
/// (+10; 0xFFFF none).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProbePolygon {
    pub area: f32,
    pub verts: [u16; 3],
    pub beside: [u16; 3],
}

/// A bound as F-35029 / F-75667 read it: its type byte (+4), its radius (+8) and centroid (+48) (the sphere's part
/// prune), its box (+16 max, +32 min, the capsule's part prune), what its material call (vtable +48) answers for each
/// index, and its shape.
#[derive(Clone, Debug)]
pub struct ProbeBound {
    pub kind: u8,
    pub radius: f32,
    pub centroid: Vec3,
    pub max: Vec3,
    pub min: Vec3,
    pub materials: Vec<u32>,
    pub shape: ProbeShape,
}

#[derive(Clone, Debug)]
pub enum ProbeShape {
    /// types 0 / 1 / 3
    Prim(Primitive),
    /// type 4 (polyhedron): the vertices (+128), the polygons (+140, their number +204), each polygon's material index
    /// (+216)
    Poly { verts: Vec<Vec3>, polys: Vec<ProbePolygon>, mats: Vec<u8> },
    /// type 5 (curved, a car's wheel; no polygons in the file): the wheel, its centroid and margin
    Curved { wheel: gjk::WheelCore, centroid: Vec3, margin: f32 },
    /// type 12: the parts (+128; none: a hole), their matrices (+132), the count the tests visit (+146) and the per-part
    /// (type, include) masks (+148; empty: none)
    Composite { parts: Vec<Option<(ProbeBound, [Vec3; 4])>>, tested: usize, masks: Vec<(u32, u32)> },
    /// type 10 (a cell of the city's grid): the vertices (+176, quantized: the file's i16 by the scale +144 plus the
    /// centre +160, F-92114 / F-54719), the polygons (+140), their material numbers (+216), the margin (+112) and the
    /// BVH (+224)
    Bvh { verts: Vec<Vec3>, polys: Vec<ProbePolygon>, mats: Vec<u8>, margin: Vec3, bvh: ProbeBvh },
    /// type 8 (the city's grid): its cells by their place in the list (+160, +164 long; none: a hole)
    Grid(ProbeGrid),
}

/// A cell's BVH as the walks F-93593 / F-54895 read it (`mcla_formats::rage::bound::CellBvh`).
#[derive(Clone, Debug)]
pub struct ProbeBvh {
    pub nodes: Vec<mcla_formats::rage::bound::BvhNode>,
    pub subtrees: Vec<mcla_formats::rage::bound::BvhSubtree>,
    /// +16 / +32 the box, +48 the quantizing origin, +64 the scale into the grid, +80 the scale out of it
    pub min: Vec3,
    pub max: Vec3,
    pub origin: Vec3,
    pub scale: Vec3,
    pub inv: Vec3,
}

/// The city's grid bound (type 8): +128 the cell size, +132 one over it, +136 / +140 the first / last column, +144 the
/// columns, +148 / +152 the first / last row, +156 the rows, +160 the cells (+164 their number), +172 / +173 the axes.
#[derive(Clone, Debug)]
pub struct ProbeGrid {
    pub cells: Vec<Option<ProbeBound>>,
    pub size: f32,
    pub inv: f32,
    pub first_x: i32,
    pub last_x: i32,
    pub columns: i32,
    pub first_z: i32,
    pub last_z: i32,
    pub rows: i32,
    pub axes: (u8, u8),
}

/// What a prop bound's material call (vtable +48) answers in the game, per index, as [`ProbeBound::of_prop`] takes it:
/// a sphere F-65278 (+144), a capsule F-60139 (+208) and a box F-90738 (+544) answer their one word whatever the index
/// (the vector repeats it past every part number the primitive tests give); a polyhedron F-10817 answers its table
/// (+208) at the index. docs/reference/people/traffic_drive_rule.md 9.11.
pub fn game_materials(b: &mcla_formats::rage::fragphys::FragBound) -> Vec<u32> {
    match b.kind {
        0 | 1 | 3 => vec![b.material; 64],
        4 => b.mat_table.clone(),
        _ => Vec::new(),
    }
}

/// A polygon's area as the original recomputes it when a car's hull is reshaped (F-89464 -> F-83274 -> F-12307): of its
/// vertices `a` (+4), `b` (+6), `c` (+8), 0.5 x |(a - b) x (c - b)|, the length as sqrt(x x + (z z + y y)).
pub fn poly_area(a: Vec3, b: Vec3, c: Vec3) -> f32 {
    let n = (a - b).cross(c - b);
    let l = (n.x as f64).mul_add(n.x as f64, (n.z as f64).mul_add(n.z as f64, (n.y * n.y) as f64) as f32 as f64) as f32;
    l.sqrt() * 0.5
}

impl ProbeBound {
    fn material(&self, i: usize) -> u32 {
        self.materials.get(i).copied().unwrap_or(0)
    }

    /// A prop type's composite bound as the dispatch reads it from the type file: a sphere by its centroid (+48) and
    /// radius (+8), a capsule by its radius +128 and length +144, a box by its centroid and size (+208), a polyhedron by
    /// its vertices, polygon records (area, vertices, neighbours) and material indices; the composite's parts with their
    /// matrices, the tested count and the part masks. `materials` gives what each bound's material call answers.
    pub fn of_prop(t: &mcla_formats::rage::fragphys::FragType, materials: &dyn Fn(&mcla_formats::rage::fragphys::FragBound) -> Vec<u32>) -> ProbeBound {
        let v = |x: [f32; 3]| Vec3::from_array(x);
        let parts = t
            .parts
            .iter()
            .map(|p| {
                let b = p.bound.as_ref()?;
                let shape = match b.kind {
                    0 => ProbeShape::Prim(Primitive::Sphere { centre: v(b.centroid), r: b.radius }),
                    1 => ProbeShape::Prim(Primitive::Capsule { centre: v(b.centroid), r: b.round, len: b.length }),
                    3 => ProbeShape::Prim(Primitive::Box { centre: v(b.centroid), size: v(b.size) }),
                    4 => ProbeShape::Poly {
                        verts: b.verts.iter().map(|&x| v(x)).collect(),
                        polys: (0..b.tris.len()).map(|i| ProbePolygon { area: b.areas[i], verts: b.tris[i], beside: b.beside[i] }).collect(),
                        mats: b.poly_mats.clone(),
                    },
                    _ => return None,
                };
                let m = [v(p.matrix[0]), v(p.matrix[1]), v(p.matrix[2]), v(p.matrix[3])];
                Some((ProbeBound { kind: b.kind, radius: b.radius, centroid: v(b.centroid), max: v(b.max), min: v(b.min), materials: materials(b), shape }, m))
            })
            .collect();
        ProbeBound {
            kind: 12,
            radius: t.radius,
            centroid: v(t.centroid),
            max: v(t.max),
            min: v(t.min),
            materials: Vec::new(),
            shape: ProbeShape::Composite { parts, tested: t.tested as usize, masks: t.part_masks.clone() },
        }
    }

    /// The city's grid bound (type 8) as the dispatches read it from the city's collision file: each cell (type 10)
    /// at its place in the grid's list with its dequantized vertices, its polygon records as the file has them (area,
    /// vertices, neighbour words), its material numbers and table (the cell's material call F-10817 answers
    /// table[number]), its margin and its BVH.
    pub fn of_city(city: &mcla_formats::rage::bound::CityBound) -> ProbeBound {
        let v = |x: [f32; 3]| Vec3::from_array(x);
        let mut cells: Vec<Option<ProbeBound>> = vec![None; city.listed as usize];
        for c in &city.cells {
            let Some(slot) = cells.get_mut(c.index) else { continue };
            let polys = (0..c.tris.len())
                .map(|i| ProbePolygon { area: c.area[i], verts: c.tris[i].map(|x| x as u16), beside: c.beside_raw[i] })
                .collect();
            let b = &c.bvh;
            let bvh = ProbeBvh { nodes: b.nodes.clone(), subtrees: b.subtrees.clone(), min: v(b.min), max: v(b.max), origin: v(b.origin), scale: v(b.scale), inv: v(b.inv) };
            *slot = Some(ProbeBound {
                kind: 10,
                radius: 0.0,
                centroid: Vec3::ZERO,
                max: Vec3::ZERO,
                min: Vec3::ZERO,
                materials: c.ids.clone(),
                shape: ProbeShape::Bvh { verts: c.verts.iter().map(|&x| v(x)).collect(), polys, mats: c.numbers.clone(), margin: v(c.margin), bvh },
            });
        }
        let grid = ProbeGrid {
            cells,
            size: city.cell_size,
            inv: city.inv_size,
            first_x: city.first_x,
            last_x: city.last_x,
            columns: city.columns,
            first_z: city.first_z,
            last_z: city.last_z,
            rows: city.rows,
            axes: city.axes,
        };
        ProbeBound { kind: 8, radius: city.radius, centroid: v(city.centroid), max: v(city.max), min: v(city.min), materials: Vec::new(), shape: ProbeShape::Grid(grid) }
    }
}

// ---- The city's grid (type 8) and its cells' polygon walks (type 10) - docs/reference/people/traffic_drive_rule.md 9.11.
// The sweep's queries (the capsule's ctor and the sphere's leave the walk cache key +2484 / +2452 and the polygon filter
// +2504 / +2472 at 0) always take F-32549's reset: slot 0, from 0, the list the driver's 4096-entry scratch (+324 /
// +292, +328 / +296 = 4096 when the query brings none).

/// The polygon list's room (the drivers' scratch, F-91312 / the sphere's driver).
pub const POLY_SCRATCH: usize = 4096;

/// vctsxs then vpkswss: a float to i32 (NaN 0, saturated), then to i16 (saturated).
fn quantum(x: f32) -> i16 {
    let i = if x.is_nan() {
        0
    } else if x >= 2_147_483_648.0 {
        i32::MAX
    } else if x < -2_147_483_648.0 {
        i32::MIN
    } else {
        x as i32
    };
    i.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

/// The vector unit's min / max as the recompiled code runs them (minps / maxps: the second operand on a NaN).
fn vmin(a: Vec3, b: Vec3) -> Vec3 {
    let m = |x: f32, y: f32| if x < y { x } else { y };
    Vec3::new(m(a.x, b.x), m(a.y, b.y), m(a.z, b.z))
}
fn vmax(a: Vec3, b: Vec3) -> Vec3 {
    let m = |x: f32, y: f32| if x > y { x } else { y };
    Vec3::new(m(a.x, b.x), m(a.y, b.y), m(a.z, b.z))
}

impl ProbeBvh {
    /// F-26111 / F-95142's quantizing: the box lo .. hi into the BVH's grid, floor((lo - origin) * scale) and
    /// ceil((hi - origin) * scale), each saturated to i16.
    fn quantized(&self, lo: Vec3, hi: Vec3) -> ([i16; 3], [i16; 3]) {
        let a = ((lo - self.origin) * self.scale).floor();
        let b = ((hi - self.origin) * self.scale).ceil();
        ([quantum(a.x), quantum(a.y), quantum(a.z)], [quantum(b.x), quantum(b.y), quantum(b.z)])
    }

    /// F-95864 / F-33493: the quantized boxes overlap on every axis (node min <= query max, query min <= node max).
    fn overlaps(q: &([i16; 3], [i16; 3]), min: &[i16; 3], max: &[i16; 3]) -> bool {
        (0..3).all(|k| min[k] <= q.1[k] && q.0[k] <= max[k])
    }

    /// F-88692: the query's quantized box lies within the node's (query min >= node min, query max <= node max).
    fn holds(q: &([i16; 3], [i16; 3]), min: &[i16; 3], max: &[i16; 3]) -> bool {
        (0..3).all(|k| q.0[k] >= min[k] && q.1[k] <= max[k])
    }

    /// F-67335: the node's box out of the grid (q * scale out + the BVH box's centre, (max + min) * 0.5, unfused)
    /// against the capsule a .. b of radius r by F-81407 (`part_dropped`: overlapping when not dropped).
    fn meets_capsule(&self, min: &[i16; 3], max: &[i16; 3], a: Vec3, b: Vec3, r: f32) -> bool {
        let f = |q: &[i16; 3]| Vec3::new(q[0] as f32, q[1] as f32, q[2] as f32);
        let centre = (self.max + self.min) * 0.5;
        let (lo, hi) = (f(min) * self.inv + centre, f(max) * self.inv + centre);
        !part_dropped(a, b, r, hi, lo)
    }

    /// The walk of every subtree header and the nodes of those the test lets through (F-93593 / F-54895): an inner
    /// node that fails skips its +12 nodes, any other goes on to the next; a leaf that passes adds its polygons
    /// (+12, +12 + 1 ..., +14 of them) while the list has room (F-44462 / F-63013).
    fn walk(&self, pass: impl Fn(&[i16; 3], &[i16; 3]) -> bool, room: usize) -> Vec<u16> {
        let mut out: Vec<u16> = Vec::new();
        for s in &self.subtrees {
            if !pass(&s.min, &s.max) {
                continue;
            }
            let (mut i, end) = (s.root as i32, s.size as i32);
            while i < end {
                let Some(n) = self.nodes.get(i as usize) else { break };
                let hit = pass(&n.min, &n.max);
                if hit {
                    for k in 0..n.count as u16 {
                        if out.len() < room {
                            out.push(n.index.wrapping_add(k));
                        }
                    }
                }
                if hit || n.count != 0 {
                    i += 1;
                } else {
                    i += n.index as i32;
                }
            }
        }
        out
    }

    /// F-94830 (F-54631's walk for the capsule a .. b of radius r): the box of its ends less / plus r (vminfp /
    /// vmaxfp), quantized; a header or node passes when the boxes overlap and the query's lies within it or the
    /// capsule meets its box (F-67335).
    pub fn capsule_polygons(&self, a: Vec3, b: Vec3, r: f32, room: usize) -> Vec<u16> {
        let q = self.quantized(vmin(a, b) - Vec3::splat(r), vmax(a, b) + Vec3::splat(r));
        self.walk(|mn, mx| Self::overlaps(&q, mn, mx) && (Self::holds(&q, mn, mx) || self.meets_capsule(mn, mx, a, b, r)), room)
    }

    /// F-33149 (F-20956's walk for the sphere c of radius r, as a box with the identity rows): the half sizes h = r +
    /// the cell's margin, the extent ((|row0 h.x| + |row1 h.y|) + |row2 h.z|), the box c -/+ it quantized; a header
    /// or node passes when the boxes overlap.
    pub fn ball_polygons(&self, c: Vec3, r: f32, margin: Vec3, room: usize) -> Vec<u16> {
        let h = Vec3::splat(r) + margin;
        let e = ((Vec3::X * h.x).abs() + (Vec3::Y * h.y).abs()) + (Vec3::Z * h.z).abs();
        let q = self.quantized(c - e, c + e);
        self.walk(|mn, mx| Self::overlaps(&q, mn, mx), room)
    }
}

/// fctiwz after the float floor: NaN the lowest word, saturated.
fn floor_word(x: f32) -> i32 {
    let f = x.floor();
    if f.is_nan() {
        i32::MIN
    } else if f >= 2_147_483_647.0 {
        i32::MAX
    } else if f < -2_147_483_648.0 {
        i32::MIN
    } else {
        f as i32
    }
}

/// The cells F-22767 / F-30918 visit for the query box lo .. hi (the query's +96 / +112), in their order: columns
/// first; the first one always, even where the box misses the grid.
#[derive(Clone, Copy, Debug)]
struct CellRange {
    x0: i32,
    z0: i32,
    x1: i32,
    z1: i32,
}

impl ProbeGrid {
    /// F-22767: x0 = max(first column, floor(lo[a0] * (1 / size))), z0 = max(first row, .. lo[a1]), x1 = min(last
    /// column, floor(hi[a0] ..)), z1 = min(last row, ..); the first cell (z0 - first row) * columns - first column + x0.
    fn first_cell(&self, lo: Vec3, hi: Vec3) -> (CellRange, i32) {
        let (a0, a1) = (self.axes.0 as usize, self.axes.1 as usize);
        let w = |v: Vec3, k: usize| floor_word(v[k.min(2)] * self.inv);
        let r = CellRange { x0: self.first_x.max(w(lo, a0)), z0: self.first_z.max(w(lo, a1)), x1: self.last_x.min(w(hi, a0)), z1: self.last_z.min(w(hi, a1)) };
        (r, (r.z0.wrapping_sub(self.first_z)).wrapping_mul(self.columns).wrapping_sub(self.first_x).wrapping_add(r.x0))
    }

    /// F-30918: the next column, or the first column of the next row; -1 past the last row.
    fn next_cell(&self, r: &CellRange, i: i32) -> i32 {
        let row = i.wrapping_div(self.columns);
        let (mut x, mut z) = (i.wrapping_sub(row.wrapping_mul(self.columns)).wrapping_add(self.first_x).wrapping_add(1), row.wrapping_add(self.first_z));
        if x > r.x1 {
            z = z.wrapping_add(1);
            x = r.x0;
            if z > r.z1 {
                return -1;
            }
        }
        (z.wrapping_sub(self.first_z)).wrapping_mul(self.columns).wrapping_sub(self.first_x).wrapping_add(x)
    }

    /// The cell at list place `i` (none past the grid or a hole).
    fn cell(&self, i: i32) -> Option<&ProbeBound> {
        if self.cells.is_empty() || i < 0 || i >= self.rows.wrapping_mul(self.columns) {
            return None;
        }
        self.cells.get(i as usize).and_then(|c| c.as_ref())
    }

    /// F-78191: the cell's box - column * size .. + size and row * size .. + size on the grid's axes, -FLT_MAX ..
    /// FLT_MAX on the third.
    fn cell_box(&self, i: i32) -> (Vec3, Vec3) {
        let row = i.wrapping_div(self.columns);
        let (x, z) = (i.wrapping_sub(row.wrapping_mul(self.columns)).wrapping_add(self.first_x), row.wrapping_add(self.first_z));
        let (fx, fz) = (x as f32 * self.size, z as f32 * self.size);
        let (mut lo, mut hi) = (Vec3::ZERO, Vec3::ZERO);
        let (a0, a1) = (self.axes.0 as usize, self.axes.1 as usize);
        lo[a0] = fx;
        hi[a0] = self.size + fx;
        lo[a1] = fz;
        hi[a1] = self.size + fz;
        let third = 3usize.wrapping_sub(a1).wrapping_sub(a0).min(2);
        lo[third] = -f32::MAX;
        hi[third] = f32::MAX;
        (lo, hi)
    }

    /// The list's box (+0 / +16) while cell `i` is dispatched: its box (F-78191) grown by 1e-6.
    fn cell_list_box(&self, i: i32) -> (Vec3, Vec3) {
        let (mn, mx) = self.cell_box(i);
        let eps = Vec3::splat(f32::from_bits(0x3586_37BD));
        (mn - eps, mx + eps)
    }
}

impl ResultList {
    /// F-15315 with 1: the new records counted into the composite level (+48 = min(max(n, 1) - +44, +48 + +52)),
    /// +52 and the new bits 0 - F-77105's count without its frame or its answer.
    pub fn count_level(&mut self) {
        let most = (self.records.len() as i32).max(1);
        self.level = (most - self.done).min(self.level + self.new);
        self.new = 0;
        self.new_bits = 0;
    }

    /// After a cell whose dispatch added records: the new bits into the level's, the records named the cell
    /// (F-19693, as F-47011) and counted into the level (F-15315 with 1).
    fn cell_done(&mut self, i: u16) {
        self.level_bits |= self.new_bits;
        self.mark_part(i);
        self.count_level();
    }

    /// After the grid's last cell: the list's box back to (0 - FLT_MAX) .. FLT_MAX, then F-67153.
    fn grid_done(&mut self) {
        (self.min, self.max) = (Vec3::ZERO - Vec3::splat(f32::MAX), Vec3::splat(f32::MAX));
        self.close_level();
    }
}

/// The undirected capsule query as the dispatch sees it: its ends +176 / +192 and radius +208 in the current frame, its
/// box +96 / +112 (the ends' smaller / larger values less / plus the radius; the list's box +0 / +16, which F-89028
/// tests, is another, left as it is), the material mask +88, the polygon index +60, the caller's f30's high word (what F-44891 saves in the stack word the
/// polygon test's line part reads), and the result list.
#[derive(Clone, Debug)]
pub struct CapsuleQuery {
    pub a: Vec3,
    pub b: Vec3,
    pub r: f32,
    pub lo: Vec3,
    pub hi: Vec3,
    pub mask: u32,
    pub index: u16,
    pub f30_high: u32,
    pub list: ResultList,
}

/// F-11472 (undirected) on a primitive or curved bound with its record: no answer, no list call; the bound's material
/// (index 0) against the mask; a slot by depth and the cap R at the contact; the record (part 0, kind 2, +80 1).
fn capsule_prim_record(q: &mut CapsuleQuery, b: &ProbeBound, inst: u32) {
    let h = match &b.shape {
        ProbeShape::Prim(p) => capsule_primitive(q.a, q.b, q.r, p),
        ProbeShape::Curved { wheel, centroid, margin } => Some(capsule_curved(q.a, q.b, q.r, wheel, *centroid, *margin)),
        _ => None,
    };
    let Some(h) = h else { return };
    let mat = b.material(0);
    if mat & q.mask != 0 {
        return;
    }
    if let Some(k) = q.list.alloc(h.depth, q.r, h.point) {
        q.list.records[k] =
            Record { instance: inst, point: h.point, normal: h.normal, bound_normal: h.normal, t: h.t, depth: h.depth, material: mat, index: 0, kind: 2, below: 1, part: 0 };
    }
}

/// F-43031 (undirected) on polygon `i` of a polygon bound with its record: the polygon's normal from its area, the
/// neighbours' from theirs (past the count: (0, 0, 1)); the material of its index against the mask; a slot by depth and
/// the cap 0 at the point; the record (index i, kind 2, +80 1).
fn capsule_poly_record(q: &mut CapsuleQuery, b: &ProbeBound, verts: &[Vec3], polys: &[ProbePolygon], mats: &[u8], i: usize, inst: u32) {
    let p = polys[i];
    let tri = |p: &ProbePolygon| [verts[p.verts[0] as usize], verts[p.verts[1] as usize], verts[p.verts[2] as usize]];
    let v = tri(&p);
    let n = gjk::polygon_normal(&v, p.area);
    let beside = |e: usize| match p.beside[e] {
        0xFFFF => None,
        j if (j as usize) < polys.len() => {
            let o = polys[j as usize];
            let ov = tri(&o);
            Some(Beside { normal: gjk::polygon_normal(&ov, o.area), v0x: Some(ov[0].x) })
        }
        _ => Some(Beside { normal: Vec3::Z, v0x: None }),
    };
    let mut slot = q.f30_high;
    let h = capsule_polygon(q.a, q.b, q.r, &v, n, n, beside, &mut slot);
    let mat = b.material(mats.get(i).copied().unwrap_or(0) as usize);
    if mat & q.mask != 0 {
        return;
    }
    if let Some(k) = q.list.alloc(h.depth, 0.0, h.point) {
        q.list.records[k] =
            Record { instance: inst, point: h.point, normal: h.normal, bound_normal: n, t: h.t, depth: h.depth, material: mat, index: i as u16, kind: 2, below: 1, part: 0 };
    }
}

/// F-35029: the undirected capsule against a bound, by its type - 0 / 1 / 3 F-11472; 4 / 5 each polygon (the query's
/// +60 its index) by F-43031, then for 5 F-11472 (the curved part); 12 each of the tested parts that is there and that
/// the part masks let through (+4 & `include`, +0 & `types`): into the part's frame (F-66128), unless the prune F-16177
/// drops it the part by this dispatch, and where that added records the new bits into the level's, the records named
/// the part (F-47011) and committed into the composite's frame (F-77105 with 1; no records: the loop ends there, the
/// query left in the part's frame), back out (F-15412); at the end the level's records become the new ones (F-67153).
/// Answers F-37493: whether new records were added. 8 the grid: each cell F-22767 / F-30918 name with the list box its own (F-78191, 1e-6 wider), by this dispatch, a cell that
/// added records named and counted into the level (F-19693, F-15315), at the end the box back and F-67153; 10 a cell: each
/// polygon its BVH walk lists (F-54631 / F-74304) by F-43031 (every city polygon is a triangle).
pub fn capsule_dispatch(q: &mut CapsuleQuery, b: &ProbeBound, inst: u32, types: u32, include: u32) -> bool {
    match (&b.shape, b.kind) {
        (ProbeShape::Prim(_), 0 | 1 | 3) => capsule_prim_record(q, b, inst),
        (ProbeShape::Poly { verts, polys, mats }, 4) => {
            for i in 0..polys.len() {
                q.index = i as u16;
                capsule_poly_record(q, b, verts, polys, mats, i, inst);
            }
        }
        (ProbeShape::Curved { .. }, 5) => capsule_prim_record(q, b, inst),
        (ProbeShape::Composite { parts, tested, masks }, 12) => {
            for i in 0..*tested {
                let Some(Some((part, m))) = parts.get(i) else { continue };
                if let Some(&(t, inc)) = masks.get(i) {
                    if inc & include == 0 || t & types == 0 {
                        continue;
                    }
                }
                (q.a, q.b, q.lo, q.hi) = part_frame(q.a, q.b, q.r, m);
                if !part_dropped(q.a, q.b, q.r, part.max, part.min) && capsule_dispatch(q, part, inst, types, include) {
                    q.list.level_bits |= q.list.new_bits;
                    q.list.mark_part(i as u16);
                    if q.list.commit(m, true) {
                        break;
                    }
                }
                (q.a, q.b, q.lo, q.hi) = part_unframe(q.a, q.b, q.r, m);
            }
            q.list.close_level();
        }
        (ProbeShape::Bvh { verts, polys, mats, bvh, .. }, 10) if !verts.is_empty() => {
            // F-54631 / F-74304: the polygons the walk F-94830 lists, in its order (+60 each one's index); every
            // polygon of the city is a triangle (kind byte +2 & 3 = 0): F-92114's vertices, F-43031 with the record
            for i in bvh.capsule_polygons(q.a, q.b, q.r, POLY_SCRATCH) {
                q.index = i;
                if (i as usize) < polys.len() {
                    capsule_poly_record(q, b, verts, polys, mats, i as usize, inst);
                }
            }
        }
        (ProbeShape::Grid(g), 8) => {
            let (r, mut i) = g.first_cell(q.lo, q.hi);
            while i != -1 {
                if let Some(c) = g.cell(i) {
                    (q.list.min, q.list.max) = g.cell_list_box(i);
                    if capsule_dispatch(q, c, inst, types, include) {
                        q.list.cell_done(i as u16);
                    }
                }
                i = g.next_cell(&r, i);
            }
            q.list.grid_done();
        }
        _ => {}
    }
    q.list.any_new()
}

/// The sphere query as its dispatch sees it: its centre +160 and radius +176 in the current frame, its box +96 / +112
/// (centre less / plus the radius), the material mask +88, the polygon index +60, the edge flags' stack slot of the
/// polygon test (F-82650 leaves it for its next call; see `ball_polygon`), and the result list.
#[derive(Clone, Debug)]
pub struct BallQuery {
    pub c: Vec3,
    pub r: f32,
    pub lo: Vec3,
    pub hi: Vec3,
    pub mask: u32,
    pub index: u16,
    pub flags: [i32; 3],
    pub list: ResultList,
}

impl BallQuery {
    /// F-22102 / F-17013's tail (the sphere's frame functions): the box +96 / +112 from the centre, per axis c - r and
    /// r + c, single rounding.
    fn boxed(&mut self) {
        self.lo = self.c - Vec3::splat(self.r);
        self.hi = Vec3::splat(self.r) + self.c;
    }
}

/// F-15615 on a primitive or curved bound with its record: the material of the part it names against the mask; a slot
/// by depth and the cap R at -(n (R - depth) - c); the record (part and kind as F-15615 names them, +80 depth < R).
fn ball_prim_record(q: &mut BallQuery, b: &ProbeBound, inst: u32) {
    let h = match &b.shape {
        ProbeShape::Prim(p) => ball_primitive(q.c, q.r, p),
        ProbeShape::Curved { wheel, centroid, margin } => ball_curved(q.c, q.r, wheel, *centroid, *margin),
        _ => return,
    };
    let mat = b.material(h.part as usize);
    if mat & q.mask != 0 {
        return;
    }
    let at = -(h.normal * (q.r - h.depth) - q.c);
    if let Some(k) = q.list.alloc(h.depth, q.r, at) {
        q.list.records[k] = Record {
            instance: inst,
            point: at,
            normal: h.normal,
            bound_normal: h.normal,
            t: 0.0,
            depth: h.depth,
            material: mat,
            index: h.part,
            kind: h.kind,
            below: (h.depth < q.r) as u8,
            part: 0,
        };
    }
}

/// F-82650 on polygon `i` of a polygon bound with its record: the polygon's normal from its area, the neighbours' from
/// theirs (past the count: (0, 0, 1)); no contact, no list call; the material of its index against the mask; a slot by
/// depth and the cap R at the contact; the record (index i, kind 2, +80 depth < R).
fn ball_poly_record(q: &mut BallQuery, b: &ProbeBound, verts: &[Vec3], polys: &[ProbePolygon], mats: &[u8], i: usize, inst: u32) {
    let p = polys[i];
    let tri = |p: &ProbePolygon| [verts[p.verts[0] as usize], verts[p.verts[1] as usize], verts[p.verts[2] as usize]];
    let v = tri(&p);
    let n = gjk::polygon_normal(&v, p.area);
    let beside = |e: usize| match p.beside[e] {
        0xFFFF => None,
        j if (j as usize) < polys.len() => {
            let o = polys[j as usize];
            Some(gjk::polygon_normal(&tri(&o), o.area))
        }
        _ => Some(Vec3::Z),
    };
    let Some(h) = ball_polygon(q.c, q.r, &v, n, n, beside, &mut q.flags) else { return };
    let mat = b.material(mats.get(i).copied().unwrap_or(0) as usize);
    if mat & q.mask != 0 {
        return;
    }
    if let Some(k) = q.list.alloc(h.depth, q.r, h.pos) {
        q.list.records[k] = Record {
            instance: inst,
            point: h.pos,
            normal: h.normal,
            bound_normal: n,
            t: 0.0,
            depth: h.depth,
            material: mat,
            index: i as u16,
            kind: 2,
            below: (h.depth < q.r) as u8,
            part: 0,
        };
    }
}

/// F-75667: the sphere against a bound, the shape of F-35029 - 0 / 1 / 3 F-15615; 4 / 5 each polygon (+60 its index) by
/// F-82650, then for 5 F-15615; 12 each tested part the masks let through: the centre into the part's frame (F-22102:
/// the rows' three-product sums of c - place) and the box, dropped (F-14017) when (the part's radius + R)^2 is under the
/// squared distance (fused z, x, then y) from its centroid - R less the lowest score held (+84) when the list's new and
/// done records (+52 + +44) fill it; else the part by this dispatch, and where that added records the level's bits, the
/// part named (F-47011) and the commit into the composite's frame (F-77105 with 1; no records: the loop ends there),
/// back out (F-17013: (x, y, z, 1) by the columns) and the box; at the end F-67153. Answers F-37493.
/// 8 the grid as in F-35029; 10 a cell: each polygon its BVH walk F-20956 / F-74304 lists (F-33149: the box c -/+ (r +
/// the cell's margin), quantized overlap only) by F-82650.
pub fn ball_dispatch(q: &mut BallQuery, b: &ProbeBound, inst: u32, types: u32, include: u32) -> bool {
    match (&b.shape, b.kind) {
        (ProbeShape::Prim(_), 0 | 1 | 3) => ball_prim_record(q, b, inst),
        (ProbeShape::Poly { verts, polys, mats }, 4) => {
            for i in 0..polys.len() {
                q.index = i as u16;
                ball_poly_record(q, b, verts, polys, mats, i, inst);
            }
        }
        (ProbeShape::Curved { .. }, 5) => ball_prim_record(q, b, inst),
        (ProbeShape::Composite { parts, tested, masks }, 12) => {
            for i in 0..*tested {
                let Some(Some((part, m))) = parts.get(i) else { continue };
                if let Some(&(t, inc)) = masks.get(i) {
                    if inc & include == 0 || t & types == 0 {
                        continue;
                    }
                }
                let w = q.c - m[3];
                q.c = Vec3::new(vdot(m[0], w), vdot(m[1], w), vdot(m[2], w));
                q.boxed();
                let reach = if q.list.new + q.list.done >= q.list.records.len() as i32 { q.r - q.list.low } else { q.r };
                let (reach, d) = (part.radius + reach, q.c - part.centroid);
                let dropped = reach * reach < fma(d.y, d.y, fma(d.z, d.z, d.x * d.x));
                if !dropped && ball_dispatch(q, part, inst, types, include) {
                    q.list.level_bits |= q.list.new_bits;
                    q.list.mark_part(i as u16);
                    if q.list.commit(m, true) {
                        break;
                    }
                }
                q.c = out_of(q.c, m);
                q.boxed();
            }
            q.list.close_level();
        }
        (ProbeShape::Bvh { verts, polys, mats, margin, bvh }, 10) if !verts.is_empty() => {
            // F-20956 / F-74304: the polygons the walk F-33149 lists, in its order; each a triangle by F-82650
            for i in bvh.ball_polygons(q.c, q.r, *margin, POLY_SCRATCH) {
                q.index = i;
                if (i as usize) < polys.len() {
                    ball_poly_record(q, b, verts, polys, mats, i as usize, inst);
                }
            }
        }
        (ProbeShape::Grid(g), 8) => {
            let (r, mut i) = g.first_cell(q.lo, q.hi);
            while i != -1 {
                if let Some(c) = g.cell(i) {
                    (q.list.min, q.list.max) = g.cell_list_box(i);
                    if ball_dispatch(q, c, inst, types, include) {
                        q.list.cell_done(i as u16);
                    }
                }
                i = g.next_cell(&r, i);
            }
            q.list.grid_done();
        }
        _ => {}
    }
    q.list.any_new()
}

/// One object of the level as the query drivers meet it (the culler's walk F-70638 / F-71552 hands the object; the
/// object table's word gives its state 0 .. 3, the state bit 1 << state): its instance word (what the records' +0 and
/// the excluded list hold), its archetype's type (+16) and include (+20) flags, its matrix (+16: rows, place) and its
/// archetype's bound (+12).
#[derive(Clone, Copy)]
pub struct LevelObject<'a> {
    pub inst: u32,
    pub state: u32,
    pub arch_type: u32,
    pub arch_include: u32,
    pub m: [Vec3; 4],
    pub bound: &'a ProbeBound,
}

/// What the level queries of the rail sweep ask: the include mask (driver r5), the states (r7; the type flags r6 are
/// -1 and the excluded types r8 0 for them) and the excluded instances (+2480 / +2448, at most 64, read by the filter
/// callbacks F-53082 / F-19966: an object in the list is skipped).
#[derive(Clone, Copy)]
pub struct LevelFilter<'a> {
    pub include: u32,
    pub states: u32,
    pub excluded: &'a [u32],
}

impl LevelFilter<'_> {
    /// The culler's test F-62425 (first form: the masks F-92940 and the bound's sphere against `vol`) and the filter
    /// callback.
    fn takes(&self, o: &LevelObject, vol: &CullVolume) -> bool {
        let state_bit = 1u32.checked_shl(o.state & 3).unwrap_or(0);
        instance_masks_pass(self.states, state_bit, self.include, u32::MAX, 0, o.arch_type, o.arch_include)
            && instance_culled_in(vol, &o.m, o.bound.centroid, o.bound.radius)
            && !self.excluded.iter().take(64).any(|&w| w == o.inst)
    }
}

/// F-91312, the swept capsule's driver (F-27454: `cap` 1 record, the material mask `mask`): the culler's volume F-21683
/// from the world ends; the list reset (F-16848, `cap` records); per object the culler keeps and the excluded list lets
/// through, in the walk's order: the ends into the object's frame (F-56283: the arithmetic of F-66128 on the world ends
/// +144 / +160, the box the ends' smaller / larger less / plus the radius), F-35029 on its archetype's bound (r6 = the
/// include mask, r7 = -1 for the part masks), and where that added records the commit F-77105 with r5 = 0 by the
/// object's matrix - a list without records ends the walk. The answer F-72810 is the list's +44 (`list.done`); the
/// sweep is blocked when the first record's instance word is set. `f30_high` is the caller's f30 (the polygon test's
/// stack word, 9.7).
pub fn capsule_level<'a>(a: Vec3, b: Vec3, r: f32, mask: u32, cap: usize, f30_high: u32, filter: &LevelFilter, objects: impl IntoIterator<Item = LevelObject<'a>>) -> CapsuleQuery {
    let vol = CullVolume::capsule(a, b, r);
    let mut q = CapsuleQuery { a, b, r, lo: Vec3::ZERO, hi: Vec3::ZERO, mask, index: 0xFFFF, f30_high, list: ResultList::new(cap) };
    for o in objects {
        if !filter.takes(&o, &vol) {
            continue;
        }
        (q.a, q.b, q.lo, q.hi) = part_frame(a, b, r, &o.m);
        if capsule_dispatch(&mut q, o.bound, o.inst, filter.include, u32::MAX) && q.list.commit(&o.m, false) {
            break;
        }
    }
    q
}

/// F-49353, the sphere query's driver (F-66549: 12 records): as `capsule_level` with the culler's ball F-36470, the
/// centre into the object's frame by F-51913 (F-22102's arithmetic on the world centre +144: rows . (c - place), the
/// box c' -/+ r) and F-75667. The edge flags' stack slot of the polygon test starts at `flags` and is carried on.
pub fn ball_level<'a>(c: Vec3, r: f32, mask: u32, cap: usize, flags: [i32; 3], filter: &LevelFilter, objects: impl IntoIterator<Item = LevelObject<'a>>) -> BallQuery {
    let vol = CullVolume::ball(c, r);
    let mut q = BallQuery { c, r, lo: Vec3::ZERO, hi: Vec3::ZERO, mask, index: 0xFFFF, flags, list: ResultList::new(cap) };
    for o in objects {
        if !filter.takes(&o, &vol) {
            continue;
        }
        let w = c - o.m[3];
        q.c = Vec3::new(vdot(o.m[0], w), vdot(o.m[1], w), vdot(o.m[2], w));
        q.boxed();
        if ball_dispatch(&mut q, o.bound, o.inst, filter.include, u32::MAX) && q.list.commit(&o.m, false) {
            break;
        }
    }
    q
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

    std::thread_local! {
        /// How often F-45690's table met each pair of the axis parameters of the edge's ends (0, 1, between), per
        /// thread (the box test calls it too).
        pub(super) static END_CASES: std::cell::RefCell<[usize; 9]> = const { std::cell::RefCell::new([0; 9]) };
        /// The last box test's feature kind and crossing count (coverage of `capsule_box_matches_the_original`).
        pub(super) static LAST_BOX: std::cell::Cell<(i32, usize)> = const { std::cell::Cell::new((0, 0)) };
        /// How often the box test took its rarer ways: the cut's same face, opposite faces, neighbours' edge,
        /// neighbours' face; the one-crossing piece kept from the start or not; a corner and an edge within 1e-12; a face
        /// of another axis than the chosen crossing's.
        pub(super) static BOX_WAYS: std::cell::RefCell<[usize; 9]> = const { std::cell::RefCell::new([0; 9]) };
    }

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
        let ends: Vec<usize> = END_CASES.with(|c| c.borrow().to_vec());
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

    /// A composite's part as the capsule query steps through it, against the original's code run offline
    /// (tools/part_prune_reference.py -> testdata/part_prune_ref.txt): F-66128 (into the part's frame), F-16177 with
    /// F-81407 / F-18932 (the part dropped) and F-15412 with F-53488 (back out), 3,000 cases (boxes through, by and off
    /// random capsules, turned and plain matrices, capsules without length): the ends and the box (x, y, z; the w lanes
    /// are lane leftovers nothing reads), the drop and the ends and box after the way back - every word equal.
    #[test]
    fn part_prune_matches_the_original() {
        let u = |h: &str| u32::from_str_radix(&h.replace('_', ""), 16).unwrap();
        let mut dropped = 0;
        for (k, line) in include_str!("../testdata/part_prune_ref.txt").lines().enumerate() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = i.split_whitespace().map(u).collect();
            let x: Vec<u32> = o.split_whitespace().map(u).collect();
            let f = |i: usize| f32::from_bits(w[i]);
            let p = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            let m = [p(0), p(3), p(6), p(9)];
            let (a, b, r) = (p(12), p(15), f(18));
            let bits4 = |v: [Vec3; 4]| v.iter().flat_map(|q| [q.x.to_bits(), q.y.to_bits(), q.z.to_bits()]).collect::<Vec<u32>>();
            let lanes = |o: usize| (0..4).flat_map(|q| x[o + 4 * q..o + 4 * q + 3].to_vec()).collect::<Vec<u32>>();
            let (fa, fb, lo, hi) = part_frame(a, b, r, &m);
            assert_eq!(bits4([fa, fb, lo, hi]), lanes(0), "line {k}: into the frame");
            let drop = part_dropped(fa, fb, r, p(19), p(22));
            assert_eq!(x[16], drop as u32, "line {k}: dropped");
            dropped += drop as usize;
            let (ba, bb, lo, hi) = part_unframe(fa, fb, r, &m);
            assert_eq!(bits4([ba, bb, lo, hi]), lanes(17), "line {k}: back");
        }
        assert_eq!(dropped, 1349);
    }

    /// The culler of a level object against the original's code run offline (tools/instance_cull_reference.py ->
    /// testdata/instance_cull_ref.txt): the culler set up from the query (F-36470 ball, F-21683 capsule) and F-62425 ->
    /// F-92940 (with F-43536), 4,000 cases (bound spheres about the query's surface, inside, far; capsules without
    /// length; masks failing each way; 1,871 pass the masks, 1,011 of them kept): the culler's volume words and the answer - every
    /// word equal.
    #[test]
    fn instance_cull_matches_the_original() {
        let u = |h: &str| u32::from_str_radix(&h.replace('_', ""), 16).unwrap();
        let (mut kept, mut near) = (0, 0);
        for (k, line) in include_str!("../testdata/instance_cull_ref.txt").lines().enumerate() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = i.split_whitespace().map(u).collect();
            let x: Vec<u32> = o.split_whitespace().map(u).collect();
            let f = |i: usize| f32::from_bits(w[i]);
            let p = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            let bits = |v: Vec3| vec![v.x.to_bits(), v.y.to_bits(), v.z.to_bits()];
            let vol = if w[0] != 0 { CullVolume::capsule(p(1), p(4), f(7)) } else { CullVolume::Ball { centre: p(1), r: f(7) } };
            match vol {
                CullVolume::Ball { centre, r } => {
                    assert_eq!((&x[0..3], &x[4..8], x[12]), (&bits(centre)[..], &[r.to_bits(); 4][..], 0), "line {k}: ball");
                }
                CullVolume::Capsule { from, dir, len, r } => {
                    assert_eq!((&x[0..3], &x[4..7], [x[8], x[9]], x[12]), (&bits(from)[..], &bits(dir)[..], [len.to_bits(), r.to_bits()], 3), "line {k}: capsule");
                }
                CullVolume::Point { .. } => panic!("line {k}: no point in this set"),
            }
            let m = [p(19), p(22), p(25), p(28)];
            let pass = instance_masks_pass(w[11], w[12], w[8], w[10], w[9], w[13], w[14]);
            let meets = instance_culled_in(&vol, &m, p(15), f(18));
            assert_eq!(x[13], (pass && meets) as u32, "line {k}: culled");
            kept += (pass && meets) as usize;
            near += pass as usize;
        }
        assert_eq!((kept, near), (1011, 1871));
    }

    /// F-36470's point volume (a sphere query's radius not above 0: 0, -0, below) and F-92940's test of it, run offline
    /// on the original's code whole (tools/instance_cull_reference.py --points --cases 1500 --seed 36470 ->
    /// testdata/instance_cull_point_ref.txt, 1,500 cases, 675 pass the masks, 329 kept): the culler's centre and type and the
    /// answer equal.
    #[test]
    fn point_cull_matches_the_original() {
        let u = |h: &str| u32::from_str_radix(&h.replace('_', ""), 16).unwrap();
        let (mut kept, mut near) = (0, 0);
        for (k, line) in include_str!("../testdata/instance_cull_point_ref.txt").lines().enumerate() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = i.split_whitespace().map(u).collect();
            let x: Vec<u32> = o.split_whitespace().map(u).collect();
            let f = |i: usize| f32::from_bits(w[i]);
            let p = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            let bits = |v: Vec3| vec![v.x.to_bits(), v.y.to_bits(), v.z.to_bits()];
            let vol = CullVolume::ball(p(1), f(7));
            let CullVolume::Point { centre } = vol else { panic!("line {k}: not a point") };
            assert_eq!((&x[0..3], x[12]), (&bits(centre)[..], 5), "line {k}: point");
            let m = [p(19), p(22), p(25), p(28)];
            let pass = instance_masks_pass(w[11], w[12], w[8], w[10], w[9], w[13], w[14]);
            let meets = instance_culled_in(&vol, &m, p(15), f(18));
            assert_eq!(x[13], (pass && meets) as u32, "line {k}: culled");
            kept += (pass && meets) as usize;
            near += pass as usize;
        }
        assert_eq!((kept, near), (329, 675));
    }

    /// F-15615 case 5 on a car's wheel against the original's code run offline (tools/ball_curved_reference.py ->
    /// testdata/ball_curved_ref.txt; F-96503 whole with the detector, the curved bound's support and the simplex routines,
    /// alloc stubbed, the material call faked as (index + 1) << 8): the result, the list call and the record equal.
    #[test]
    fn ball_curved_matches_the_original() {
        let u = |h: &str| u32::from_str_radix(&h.replace('_', ""), 16).unwrap();
        let (mut hits, mut deep, mut none, mut far) = (0, 0, 0, 0);
        for (k, line) in include_str!("../testdata/ball_curved_ref.txt").lines().enumerate() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = i.split_whitespace().map(u).collect();
            let x: Vec<u32> = o.split_whitespace().map(u).collect();
            let f = |i: usize| f32::from_bits(w[i]);
            let (r, wd) = (f(0), f(1));
            let shape = gjk::WheelShape::sized(&[(f32::from_bits(0x3e99_999a), f32::from_bits(0x3dcc_cccd)), (r, wd)]);
            let wheel = gjk::WheelCore::of(&shape, Vec3::ZERO, [Vec3::X, Vec3::Y, Vec3::Z], r, wd);
            let (c, big_r) = (Vec3::new(f(2), f(3), f(4)), f(5));
            let h = ball_curved(c, big_r, &wheel, Vec3::ZERO, gjk::WHEEL_MARGIN);
            match h.depth {
                d if d == -1.0 => far += 1,
                d if d == 0.0 => none += 1,
                d if d > big_r => deep += 1,
                _ => hits += 1,
            }
            let bits = |x: Vec3| [x.x.to_bits(), x.y.to_bits(), x.z.to_bits()];
            let mat = 1u32 << 8;
            if mat & w[6] != 0 {
                assert_eq!(&x[0..2], &[0, 0], "line {k}: material");
                continue;
            }
            let at = -(h.normal * (big_r - h.depth) - c);
            assert_eq!([x[0], x[1]], [(h.depth > 0.0) as u32, 1], "line {k}: result, list call");
            assert_eq!([x[2], x[3]], [h.depth.to_bits(), big_r.to_bits()], "line {k}: depth, cap");
            assert_eq!(&x[4..7], &bits(at), "line {k}: point");
            if w[7] != 0 && h.depth > 0.0 {
                assert_eq!(x[7], 0x1234, "line {k}: instance");
                assert_eq!(&x[8..11], &bits(at), "line {k}: contact");
                assert_eq!(&x[11..14], &bits(h.normal), "line {k}: normal");
                assert_eq!(&x[14..17], &bits(h.normal), "line {k}: bound normal");
                assert_eq!(&x[17..20], &[0, h.depth.to_bits(), mat], "line {k}: t, depth, material");
                assert_eq!(&x[20..24], &[0, 2, (h.depth < big_r) as u32, 0], "line {k}: part, kind, +80, +82");
            }
        }
        eprintln!("ball_curved: {hits} touching, {deep} deeper than the radius, {none} at depth 0, {far} none or facing away");
        assert_eq!((hits, deep, none, far), (1844, 1062, 0, 1094));
    }

    /// F-11472 undirected on a car's wheel against the original's code run offline (tools/capsule_curved_reference.py ->
    /// testdata/capsule_curved_ref.txt; the capsule bound F-25220 / F-77704, F-96503 whole with the detector, the curved
    /// bound's support, the capsule's and the simplex routines, F-36467; alloc stubbed, the material call faked as
    /// (index + 1) << 8): the result, the list call (depth, cap, point) and the record equal.
    #[test]
    fn capsule_curved_matches_the_original() {
        let u = |h: &str| u32::from_str_radix(&h.replace('_', ""), 16).unwrap();
        let (mut hits, mut deep, mut none, mut zero_len) = (0, 0, 0, 0);
        let mut zero_ways = std::collections::BTreeMap::new();
        for (k, line) in include_str!("../testdata/capsule_curved_ref.txt").lines().enumerate() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = i.split_whitespace().map(u).collect();
            let x: Vec<u32> = o.split_whitespace().map(u).collect();
            let f = |i: usize| f32::from_bits(w[i]);
            let p = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            let (r, wd) = (f(0), f(1));
            let shape = gjk::WheelShape::sized(&[(f32::from_bits(0x3e99_999a), f32::from_bits(0x3dcc_cccd)), (r, wd)]);
            let wheel = gjk::WheelCore::of(&shape, Vec3::ZERO, [Vec3::X, Vec3::Y, Vec3::Z], r, wd);
            let (a, b, big_r) = (p(2), p(5), f(8));
            let h = capsule_curved(a, b, big_r, &wheel, Vec3::ZERO, gjk::WHEEL_MARGIN);
            zero_len += (a == b) as usize;
            if h.depth == 0.0 {
                // (which way to depth 0: F-96503 without an answer, its normal facing away, or its two points together)
                let ways = if h.normal == Vec3::ZERO && h.t == 0.0 { 0 } else if h.normal.length_squared() > 1.5 || h.normal.length() < 0.5 { 1 } else { 2 };
                *zero_ways.entry(ways).or_insert(0) += 1;
            }
            match h.depth {
                d if d == 0.0 => none += 1,
                d if d > big_r => deep += 1,
                _ => hits += 1,
            }
            let bits = |x: Vec3| [x.x.to_bits(), x.y.to_bits(), x.z.to_bits()];
            if 256 & w[9] != 0 {
                assert_eq!(&x[0..2], &[0, 0], "line {k}: material");
                continue;
            }
            assert_eq!([x[0], x[1]], [(h.depth > 0.0) as u32, 1], "line {k}: result, list call");
            assert_eq!([x[2], x[3]], [h.depth.to_bits(), big_r.to_bits()], "line {k}: depth, cap");
            assert_eq!(&x[4..7], &bits(h.point), "line {k}: point");
            if w[10] != 0 && h.depth > 0.0 {
                assert_eq!(x[7], 0x1234, "line {k}: instance");
                assert_eq!(&x[8..11], &bits(h.point), "line {k}: contact");
                assert_eq!(&x[11..14], &bits(h.normal), "line {k}: normal");
                assert_eq!(&x[14..17], &bits(h.normal), "line {k}: bound normal");
                assert_eq!(&x[17..20], &[h.t.to_bits(), h.depth.to_bits(), 256], "line {k}: t, depth, material");
                assert_eq!(&x[20..24], &[0, 2, 1, 0], "line {k}: part, kind, +80, +82");
            }
        }
        eprintln!("capsule_curved: {hits} touching, {deep} deeper than the radius, {none} at depth 0 ({zero_len} without length) {zero_ways:?}");
        assert_eq!((hits, deep, none, zero_len), (1827, 1246, 927, 194));
    }

    /// F-15615 on spheres, capsules and boxes against the original's code run offline (tools/ball_primitive_reference.py
    /// -> testdata/ball_primitive_ref.txt; F-25936 whole, alloc stubbed, the material call faked as (index + 1) << 8): the
    /// result, the list call (depth, cap, point) and the record - every word equal. 4,000 cases: 784 spheres, 807 capsules
    /// (on the axis among them), boxes with the ball inside 467, by a corner 487 (50 on it), by an edge 774 (94 on its
    /// line), over a face 681.
    #[test]
    fn ball_primitive_matches_the_original() {
        let u = |h: &str| u32::from_str_radix(&h.replace('_', ""), 16).unwrap();
        let mut seen = std::collections::BTreeMap::new();
        for (k, line) in include_str!("../testdata/ball_primitive_ref.txt").lines().enumerate() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = i.split_whitespace().map(u).collect();
            let x: Vec<u32> = o.split_whitespace().map(u).collect();
            let f = |i: usize| f32::from_bits(w[i]);
            let p = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            let b = match w[0] {
                0 => Primitive::Sphere { centre: p(1), r: f(4) },
                1 => Primitive::Capsule { centre: p(1), r: f(5), len: f(6) },
                _ => Primitive::Box { centre: p(1), size: p(7) },
            };
            let (c, big_r) = (p(10), f(13));
            let h = ball_primitive(c, big_r, &b);
            let bits = |x: Vec3| [x.x.to_bits(), x.y.to_bits(), x.z.to_bits()];
            let mat = (h.part as u32 + 1) << 8;
            // (coverage: the box's feature by the classifier's regions; a ball on its feature)
            let feature = match b {
                Primitive::Box { centre, size } => {
                    let (d, hh) = (c - centre, size * 0.5);
                    let reg = |p: f32, h: f32| if !(p > -h) { 0 } else if p < h { 1 } else { 2 };
                    BOX_FEATURE[reg(d.x, hh.x) + 3 * reg(d.y, hh.y) + 9 * reg(d.z, hh.z)].0
                }
                _ => 9,
            };
            let on = h.depth == big_r || h.normal == Vec3::X && h.depth > 0.0 && w[0] != 3;
            *seen.entry((w[0], feature, h.depth > 0.0, on)).or_insert(0) += 1;
            if mat & w[14] != 0 {
                assert_eq!(&x[0..2], &[0, 0], "line {k}: material");
                continue;
            }
            let at = -(h.normal * (big_r - h.depth) - c);
            assert_eq!([x[0], x[1]], [(h.depth > 0.0) as u32, 1], "line {k}: result, list call");
            assert_eq!([x[2], x[3]], [h.depth.to_bits(), big_r.to_bits()], "line {k}: depth, cap");
            assert_eq!(&x[4..7], &bits(at), "line {k}: point");
            if w[15] != 0 && h.depth > 0.0 {
                assert_eq!(x[7], 0x1234, "line {k}: instance");
                assert_eq!(&x[8..11], &bits(at), "line {k}: contact");
                assert_eq!(&x[11..14], &bits(h.normal), "line {k}: normal");
                assert_eq!(&x[14..17], &bits(h.normal), "line {k}: bound normal");
                assert_eq!(&x[17..20], &[0, h.depth.to_bits(), mat], "line {k}: t, depth, material");
                assert_eq!(&x[20..24], &[h.part as u32, h.kind as u32, (h.depth < big_r) as u32, 0], "line {k}: part, kind, +80, +82");
            }
        }
        assert_eq!(
            format!("{seen:?}"),
            "{(0, 9, false, false): 306, (0, 9, true, false): 290, (0, 9, true, true): 188, (1, 9, false, false): 457, (1, 9, true, false): 318, \
             (1, 9, true, true): 32, (3, -1, true, false): 466, (3, -1, true, true): 1, (3, 0, false, false): 227, (3, 0, true, false): 210, \
             (3, 0, true, true): 50, (3, 1, false, false): 195, (3, 1, true, false): 485, (3, 1, true, true): 94, (3, 2, false, false): 133, \
             (3, 2, true, false): 523, (3, 2, true, true): 25}"
        );
    }

    /// F-11472 (undirected) on spheres and capsules against the original's code run offline
    /// (tools/capsule_primitive_reference.py -> testdata/capsule_primitive_ref.txt; F-76120, F-40164 with F-98480 and
    /// F-43156, F-95825 and F-41334 whole; alloc stubbed, the material call faked): the result, the list call and the
    /// record - every word equal. 4,000 cases: 1,329 spheres, 2,671 capsules (1,029 by the quick axis test).
    #[test]
    fn capsule_primitive_matches_the_original() {
        let u = |h: &str| u32::from_str_radix(&h.replace('_', ""), 16).unwrap();
        let mut seen = std::collections::BTreeMap::new();
        for (k, line) in include_str!("../testdata/capsule_primitive_ref.txt").lines().enumerate() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = i.split_whitespace().map(u).collect();
            let x: Vec<u32> = o.split_whitespace().map(u).collect();
            let f = |i: usize| f32::from_bits(w[i]);
            let p = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            let b = match w[0] {
                0 => Primitive::Sphere { centre: p(1), r: f(4) },
                _ => Primitive::Capsule { centre: p(1), r: f(5), len: f(6) },
            };
            let (a, bb, big_r) = (p(7), p(10), f(13));
            let h = capsule_primitive(a, bb, big_r, &b).unwrap();
            let bits = |x: Vec3| [x.x.to_bits(), x.y.to_bits(), x.z.to_bits()];
            // (coverage: the type, the quick answer of the capsule's axis test, a normal of its own, a hit)
            let quick = match b {
                Primitive::Capsule { centre, len, .. } => axis_quick(a - centre, bb - a, -(len * 0.5), len * 0.5).is_some(),
                _ => false,
            };
            *seen.entry((w[0], quick, h.depth == f(if w[0] == 0 { 4 } else { 5 }) + big_r, h.depth > 0.0)).or_insert(0) += 1;
            if 256 & w[14] != 0 {
                assert_eq!(&x[0..2], &[0, 0], "line {k}: material");
                continue;
            }
            assert_eq!([x[0], x[1]], [(h.depth > 0.0) as u32, 1], "line {k}: result, list call");
            assert_eq!([x[2], x[3]], [h.depth.to_bits(), big_r.to_bits()], "line {k}: depth, cap");
            assert_eq!(&x[4..7], &bits(h.point), "line {k}: point");
            if w[15] != 0 && h.depth > 0.0 {
                assert_eq!(x[7], 0x1234, "line {k}: instance");
                assert_eq!(&x[8..11], &bits(h.point), "line {k}: contact");
                assert_eq!(&x[11..14], &bits(h.normal), "line {k}: normal");
                assert_eq!(&x[14..17], &bits(h.normal), "line {k}: bound normal");
                assert_eq!(&x[17..20], &[h.t.to_bits(), h.depth.to_bits(), 256], "line {k}: t, depth, material");
                assert_eq!(&x[20..24], &[0, 2, 1, 0], "line {k}: part, kind, +80, +82");
            }
        }
        assert_eq!(
            format!("{seen:?}"),
            "{(0, false, false, false): 267, (0, false, false, true): 486, (0, false, true, true): 576, (1, false, false, false): 671, \
             (1, false, false, true): 538, (1, false, true, true): 433, (1, true, false, false): 498, (1, true, false, true): 491, \
             (1, true, true, true): 40}"
        );
    }

    /// F-11472 (undirected) on boxes against the original's code run offline (tools/capsule_box_reference.py ->
    /// testdata/capsule_box_ref.txt; F-79106 with F-25936, F-21753, F-94282, F-36467, F-45690 with F-89261 whole; alloc
    /// stubbed, the material call faked): whether a record is made, the result, the list call and the record - every
    /// word equal. 4,000 cases by (feature, crossings 0 / 1 / more, a record with depth above 0): the whole axis inside
    /// 252; corners 621, edges 1,676, faces 1,451; 721 without a list call; the rarer ways counted in `BOX_WAYS`.
    #[test]
    fn capsule_box_matches_the_original() {
        let u = |h: &str| u32::from_str_radix(&h.replace('_', ""), 16).unwrap();
        let mut seen = std::collections::BTreeMap::new();
        for (k, line) in include_str!("../testdata/capsule_box_ref.txt").lines().enumerate() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = i.split_whitespace().map(u).collect();
            let x: Vec<u32> = o.split_whitespace().map(u).collect();
            let f = |i: usize| f32::from_bits(w[i]);
            let p = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            let b = Primitive::Box { centre: p(1), size: p(4) };
            let (a, bb, big_r) = (p(7), p(10), f(13));
            let h = capsule_primitive(a, bb, big_r, &b);
            let (kind, n) = LAST_BOX.with(|c| c.get());
            *seen.entry((kind, n.min(2), h.map(|h| h.depth > 0.0))).or_insert(0) += 1;
            let Some(h) = h else {
                assert_eq!(&x[0..2], &[0, 0], "line {k}: no record");
                continue;
            };
            let bits = |x: Vec3| [x.x.to_bits(), x.y.to_bits(), x.z.to_bits()];
            if 256 & w[14] != 0 {
                assert_eq!(&x[0..2], &[0, 0], "line {k}: material");
                continue;
            }
            assert_eq!([x[0], x[1]], [(h.depth > 0.0) as u32, 1], "line {k}: result, list call");
            assert_eq!([x[2], x[3]], [h.depth.to_bits(), big_r.to_bits()], "line {k}: depth, cap");
            assert_eq!(&x[4..7], &bits(h.point), "line {k}: point");
            if w[15] != 0 && h.depth > 0.0 {
                assert_eq!(x[7], 0x1234, "line {k}: instance");
                assert_eq!(&x[8..11], &bits(h.point), "line {k}: contact");
                assert_eq!(&x[11..14], &bits(h.normal), "line {k}: normal");
                assert_eq!(&x[14..17], &bits(h.normal), "line {k}: bound normal");
                assert_eq!(&x[17..20], &[h.t.to_bits(), h.depth.to_bits(), 256], "line {k}: t, depth, material");
                assert_eq!(&x[20..24], &[0, 2, 1, 0], "line {k}: part, kind, +80, +82");
            }
        }
        assert_eq!(BOX_WAYS.with(|w| *w.borrow()), [275, 272, 554, 137, 469, 596, 134, 313, 662]);
        assert_eq!(
            format!("{seen:?}"),
            "{(-1, 0, Some(true)): 252, (0, 0, None): 151, (0, 0, Some(true)): 133, (0, 1, None): 46, (0, 1, Some(true)): 66, \
             (0, 2, None): 68, (0, 2, Some(true)): 157, (1, 0, None): 135, (1, 0, Some(true)): 163, (1, 1, None): 108, \
             (1, 1, Some(true)): 396, (1, 2, None): 71, (1, 2, Some(true)): 803, (2, 0, None): 48, (2, 0, Some(true)): 188, \
             (2, 1, None): 44, (2, 1, Some(false)): 41, (2, 1, Some(true)): 364, (2, 2, None): 50, (2, 2, Some(false)): 17, \
             (2, 2, Some(true)): 699}"
        );
    }

    /// F-35029 with the result list against the original's code run offline on the props' composites (tools/
    /// capsule_dispatch_reference.py -> testdata/capsule_dispatch_ref.txt; the list's reset F-16848, F-35029 with its
    /// composite loop, F-11472, F-43031, F-89028 and F-77105 whole, only the material call faked as (index + 1) << 8):
    /// per line the list's records, the caller's f30, then one to three dispatches (the type, the capsule, its box, the
    /// material mask, the commit's matrix), each committed with r5 = 0 when it answered; out every answer, the query's
    /// ends and box, the list's counts, bits and scores and every record - all equal. The prop file is read by the
    /// port's reader (fragphys), which the original reads in the reference laid out as it is.
    /// The words the prop bounds' material calls answer (`game_materials`), read from the prop type file as the
    /// original's getters read them (F-65278 +144, F-60139 +208, F-90738 +544, F-10817 the table +208): counted per class
    /// and word, equal to an independent reading of the same bytes (work/traffic-detail/fix62/py/box544.py). The sweep's
    /// mask 0x28000 drops only the 77 capsules whose word 0x2003A has 0x20000 (push_sphere << 8).
    #[test]
    fn prop_material_words_are_the_files() {
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let Ok(d) = std::fs::read(std::path::Path::new(&root).join("resources/prop/0x0e66973c.r131")) else { return };
        if d.len() < 16 || &d[0..4] != b"MCLR" {
            return;
        }
        let u32le = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let data = d[16..].to_vec();
        let vs = (u32le(12) as usize).min(data.len());
        let res = mcla_formats::rsc::Resource { rtype: u32le(4), flags: u32le(8), virtual_size: vs, physical_size: data.len() - vs, data };
        let types = mcla_formats::rage::fragphys::parse_frag_types(&res);
        let mut got = std::collections::BTreeMap::new();
        for t in &types {
            for b in t.parts.iter().filter_map(|p| p.bound.as_ref()) {
                let w = game_materials(b);
                *got.entry((b.kind, w.first().copied().unwrap_or(u32::MAX))).or_insert(0) += 1;
            }
        }
        println!("{got:?}");
        let want = [((0u8, 0u32), 31), ((0, 0x1d), 63), ((1, 0), 11), ((1, 0x1d), 129), ((1, 0x2_003a), 77), ((3, 0), 11), ((3, 0x1d), 367), ((4, 0x1d), 15)];
        assert_eq!(got.into_iter().collect::<Vec<_>>(), want.to_vec());
        let dropped = types.iter().flat_map(|t| t.parts.iter().filter_map(|p| p.bound.as_ref())).filter(|b| game_materials(b).iter().any(|&w| w & 0x28000 != 0)).count();
        assert_eq!(dropped, 77);
    }

    #[test]
    fn capsule_dispatch_matches_the_original() {
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let Ok(d) = std::fs::read(std::path::Path::new(&root).join("resources/prop/0x0e66973c.r131")) else { return };
        if d.len() < 16 || &d[0..4] != b"MCLR" {
            return;
        }
        let u32le = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let data = d[16..].to_vec();
        let vs = (u32le(12) as usize).min(data.len());
        let res = mcla_formats::rsc::Resource { rtype: u32le(4), flags: u32le(8), virtual_size: vs, physical_size: data.len() - vs, data };
        let types = mcla_formats::rage::fragphys::parse_frag_types(&res);
        let fake = |b: &mcla_formats::rage::fragphys::FragBound| -> Vec<u32> {
            if b.kind == 4 { (0..256).map(|j| (j + 1) << 8).collect() } else { vec![256] }
        };
        let bounds: Vec<ProbeBound> = types.iter().map(|t| ProbeBound::of_prop(t, &fake)).collect();
        let u = |h: &str| u32::from_str_radix(&h.replace('_', ""), 16).unwrap();
        let bits = |x: Vec3| [x.x.to_bits(), x.y.to_bits(), x.z.to_bits()];
        let (mut hits, mut full, mut polys, mut recs, mut poly_recs) = (0, 0, 0, 0, 0);
        for (k, line) in include_str!("../testdata/capsule_dispatch_ref.txt").lines().enumerate() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let mut groups = i.split(" | ");
            let head: Vec<u32> = groups.next().unwrap().split_whitespace().map(u).collect();
            let x: Vec<u32> = o.split_whitespace().map(u).collect();
            let cap = head[0] as usize;
            let mut q = CapsuleQuery { a: Vec3::ZERO, b: Vec3::ZERO, r: 0.0, lo: Vec3::ZERO, hi: Vec3::ZERO, mask: 0, index: 0xFFFF, f30_high: head[1], list: ResultList::new(cap) };
            let mut got = Vec::new();
            for g in groups {
                let w: Vec<u32> = g.split_whitespace().map(u).collect();
                let f = |i: usize| f32::from_bits(w[i]);
                let p = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
                (q.a, q.b, q.r, q.list.min, q.list.max, q.mask) = (p(1), p(4), f(7), p(8), p(11), w[14]);
                let m = [p(15), p(18), p(21), p(24)];
                let bound = &bounds[w[0] as usize];
                polys += matches!(&bound.shape, ProbeShape::Composite { parts, .. } if parts.iter().flatten().any(|p| p.0.kind == 4)) as usize;
                let r1 = capsule_dispatch(&mut q, bound, 0x1234, u32::MAX, u32::MAX);
                let r2 = if r1 { q.list.commit(&m, false) } else { false };
                got.extend([r1 as u32, r2 as u32]);
                hits += r1 as usize;
            }
            let n = got.len();
            assert_eq!(&x[..n], &got[..], "line {k}: answers");
            let l = &q.list;
            let mut st = Vec::new();
            for v in [q.a, q.b, l.min, l.max, q.lo, q.hi] {
                st.extend(bits(v));
            }
            st.extend([l.done as u32, l.level as u32, l.new as u32, l.new_bits as u32, (l.new_bits >> 32) as u32, l.level_bits as u32, (l.level_bits >> 32) as u32]);
            st.extend([l.high.to_bits(), l.low.to_bits()]);
            assert_eq!(&x[n..n + 27], &st[..], "line {k}: query and list");
            full += (cap > 0 && l.done as usize == cap) as usize;
            for (j, r) in l.records.iter().enumerate() {
                let y = &x[n + 27 + 17 * j..n + 44 + 17 * j];
                let mut e = vec![r.instance];
                for v in [r.point, r.normal, r.bound_normal] {
                    e.extend(bits(v));
                }
                e.extend([r.t.to_bits(), r.depth.to_bits(), r.material, r.index as u32, r.kind as u32, r.below as u32, r.part as u32]);
                assert_eq!(y, &e[..], "line {k}: record {j}");
                recs += (r.instance != 0) as usize;
                poly_recs += (r.instance != 0 && r.index > 0) as usize;
            }
        }
        // (dispatches that added records, lists left full, dispatches on a type with a polyhedron, records, records of a
        // polygon past the first)
        assert_eq!((hits, full, polys, recs, poly_recs), (2050, 1009, 1745, 2177, 967), "coverage");
    }

    /// F-75667 with the result list against the original's code run offline on the props' composites (tools/
    /// ball_dispatch_reference.py -> testdata/ball_dispatch_ref.txt; F-16848, F-75667 with its composite loop, F-15615,
    /// F-82650, F-89028 and F-77105 whole, the material call faked): the sphere's centre, radius, the list's box, the
    /// mask and the commit's matrix per dispatch; every answer, the centre, both boxes, the list and every record equal.
    /// The edge flags' stack slot starts at 0 (the harness's stack) and is carried from call to call.
    #[test]
    fn ball_dispatch_matches_the_original() {
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let Ok(d) = std::fs::read(std::path::Path::new(&root).join("resources/prop/0x0e66973c.r131")) else { return };
        if d.len() < 16 || &d[0..4] != b"MCLR" {
            return;
        }
        let u32le = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let data = d[16..].to_vec();
        let vs = (u32le(12) as usize).min(data.len());
        let res = mcla_formats::rsc::Resource { rtype: u32le(4), flags: u32le(8), virtual_size: vs, physical_size: data.len() - vs, data };
        let types = mcla_formats::rage::fragphys::parse_frag_types(&res);
        let fake = |_: &mcla_formats::rage::fragphys::FragBound| -> Vec<u32> { (0..256).map(|j| (j + 1) << 8).collect() };
        let bounds: Vec<ProbeBound> = types.iter().map(|t| ProbeBound::of_prop(t, &fake)).collect();
        let u = |h: &str| u32::from_str_radix(&h.replace('_', ""), 16).unwrap();
        let bits = |x: Vec3| [x.x.to_bits(), x.y.to_bits(), x.z.to_bits()];
        let (mut hits, mut full, mut recs, mut poly_recs) = (0, 0, 0, 0);
        for (k, line) in include_str!("../testdata/ball_dispatch_ref.txt").lines().enumerate() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let mut groups = i.split(" | ");
            let head: Vec<u32> = groups.next().unwrap().split_whitespace().map(u).collect();
            let x: Vec<u32> = o.split_whitespace().map(u).collect();
            let cap = head[0] as usize;
            let mut q = BallQuery { c: Vec3::ZERO, r: 0.0, lo: Vec3::ZERO, hi: Vec3::ZERO, mask: 0, index: 0xFFFF, flags: [0; 3], list: ResultList::new(cap) };
            let mut got = Vec::new();
            for g in groups {
                let w: Vec<u32> = g.split_whitespace().map(u).collect();
                let f = |i: usize| f32::from_bits(w[i]);
                let p = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
                (q.c, q.r, q.list.min, q.list.max, q.mask) = (p(1), f(7), p(8), p(11), w[14]);
                q.boxed();
                let m = [p(15), p(18), p(21), p(24)];
                let r1 = ball_dispatch(&mut q, &bounds[w[0] as usize], 0x1234, u32::MAX, u32::MAX);
                let r2 = if r1 { q.list.commit(&m, false) } else { false };
                got.extend([r1 as u32, r2 as u32]);
                hits += r1 as usize;
            }
            let n = got.len();
            assert_eq!(&x[..n], &got[..], "line {k}: answers");
            let l = &q.list;
            let mut st = Vec::new();
            for v in [q.c, l.min, l.max, q.lo, q.hi] {
                st.extend(bits(v));
            }
            st.extend([l.done as u32, l.level as u32, l.new as u32, l.new_bits as u32, (l.new_bits >> 32) as u32, l.level_bits as u32, (l.level_bits >> 32) as u32]);
            st.extend([l.high.to_bits(), l.low.to_bits()]);
            assert_eq!(&x[n..n + 24], &st[..], "line {k}: query and list");
            full += (cap > 0 && l.done as usize == cap) as usize;
            for (j, r) in l.records.iter().enumerate() {
                let y = &x[n + 24 + 17 * j..n + 41 + 17 * j];
                let mut e = vec![r.instance];
                for v in [r.point, r.normal, r.bound_normal] {
                    e.extend(bits(v));
                }
                e.extend([r.t.to_bits(), r.depth.to_bits(), r.material, r.index as u32, r.kind as u32, r.below as u32, r.part as u32]);
                assert_eq!(y, &e[..], "line {k}: record {j}");
                recs += (r.instance != 0) as usize;
                poly_recs += (r.instance != 0 && r.bound_normal != r.normal) as usize;
            }
        }
        // (dispatches that added records, lists left full, records, polygon records by an edge or a corner)
        assert_eq!((hits, full, recs, poly_recs), (1332, 654, 1425, 557), "coverage");
    }

    fn city_probe() -> Option<ProbeBound> {
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let d = std::fs::read(std::path::Path::new(&root).join("resources/city/sc/0x3e5fd91d.r1")).ok()?;
        if d.len() < 16 || &d[0..4] != b"MCLR" {
            return None;
        }
        let u32le = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let data = d[16..].to_vec();
        let vs = (u32le(12) as usize).min(data.len());
        let res = mcla_formats::rsc::Resource { rtype: u32le(4), flags: u32le(8), virtual_size: vs, physical_size: data.len() - vs, data };
        Some(ProbeBound::of_city(&mcla_formats::rage::bound::CityBound::parse(&res).unwrap()))
    }

    /// The cells' polygon walks against the original's F-54631 / F-20956 run on the city's collision file
    /// (tools/city_walk_reference.py): the count, the slot, the first index and the whole list, in order.
    #[test]
    fn city_walk_matches_the_original() {
        let Some(city) = city_probe() else { return };
        let ProbeShape::Grid(g) = &city.shape else { panic!() };
        let u = |h: &str| u32::from_str_radix(&h.replace('_', ""), 16).unwrap();
        let (mut listed, mut most, mut cut) = (0usize, 0usize, 0usize);
        for (k, line) in include_str!("../testdata/city_walk_ref.txt").lines().enumerate() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = i.split_whitespace().map(u).collect();
            let x: Vec<u32> = o.split_whitespace().map(u).collect();
            let f = |i: usize| f32::from_bits(w[i]);
            let p = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            let cell = g.cells[w[1] as usize].as_ref().unwrap();
            let ProbeShape::Bvh { margin, bvh, .. } = &cell.shape else { panic!() };
            let list = if w[0] != 0 { bvh.ball_polygons(p(2), f(8), *margin, POLY_SCRATCH) } else { bvh.capsule_polygons(p(2), p(5), f(8), POLY_SCRATCH) };
            let first = list.first().map_or(0xFFFF_FFFF, |&i| i as u32);
            let mut e = vec![list.len() as u32, 0, list.first().copied().unwrap_or(0) as u32, first, list.len() as u32];
            e.extend(list.iter().map(|&i| i as u32));
            assert_eq!(x, e, "line {k}");
            listed += list.len();
            most = most.max(list.len());
            // (cases where the capsule's float test, not the quantized overlap, decided)
            if w[0] == 0 {
                let (a, b, r) = (p(2), p(5), f(8));
                let q = bvh.quantized(vmin(a, b) - Vec3::splat(r), vmax(a, b) + Vec3::splat(r));
                cut += (bvh.walk(|mn, mx| ProbeBvh::overlaps(&q, mn, mx), POLY_SCRATCH).len() != list.len()) as usize;
            }
        }
        assert_eq!((listed, most, cut), (100179, 288, 236), "coverage");
    }

    /// The city's grid with its cells and their BVH walks (types 8 / 10) against the original's F-35029 / F-75667 run
    /// whole on the city's collision file (tools/city_dispatch_reference.py): every answer, the query, the list's box,
    /// counts, bits and scores and every record equal.
    #[test]
    fn city_dispatch_matches_the_original() {
        let Some(city) = city_probe() else { return };
        let u = |h: &str| u32::from_str_radix(&h.replace('_', ""), 16).unwrap();
        let bits = |x: Vec3| [x.x.to_bits(), x.y.to_bits(), x.z.to_bits()];
        let id = [Vec3::X, Vec3::Y, Vec3::Z, Vec3::ZERO];
        let (mut hits, mut full, mut recs, mut cells_hit, mut balls) = (0, 0, 0, std::collections::BTreeSet::new(), 0);
        for (k, line) in include_str!("../testdata/city_dispatch_ref.txt").lines().enumerate() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let mut groups = i.split(" | ");
            let head: Vec<u32> = groups.next().unwrap().split_whitespace().map(u).collect();
            let x: Vec<u32> = o.split_whitespace().map(u).collect();
            let (ball, cap) = (head[0] != 0, head[1] as usize);
            balls += ball as usize;
            let mut cq = CapsuleQuery { a: Vec3::ZERO, b: Vec3::ZERO, r: 0.0, lo: Vec3::ZERO, hi: Vec3::ZERO, mask: 0, index: 0xFFFF, f30_high: head[2], list: ResultList::new(cap) };
            let mut bq = BallQuery { c: Vec3::ZERO, r: 0.0, lo: Vec3::ZERO, hi: Vec3::ZERO, mask: 0, index: 0xFFFF, flags: [0; 3], list: ResultList::new(cap) };
            let mut got = Vec::new();
            for g in groups {
                let w: Vec<u32> = g.split_whitespace().map(u).collect();
                let f = |i: usize| f32::from_bits(w[i]);
                let p = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
                let (r1, r2) = if ball {
                    (bq.c, bq.r, bq.mask) = (p(0), f(6), w[7]);
                    bq.boxed();
                    let r1 = ball_dispatch(&mut bq, &city, 0x1234, u32::MAX, u32::MAX);
                    (r1, r1 && bq.list.commit(&id, false))
                } else {
                    (cq.a, cq.b, cq.r, cq.mask) = (p(0), p(3), f(6), w[7]);
                    (cq.lo, cq.hi) = (vmin(cq.a, cq.b) - Vec3::splat(cq.r), vmax(cq.a, cq.b) + Vec3::splat(cq.r));
                    let r1 = capsule_dispatch(&mut cq, &city, 0x1234, u32::MAX, u32::MAX);
                    (r1, r1 && cq.list.commit(&id, false))
                };
                got.extend([r1 as u32, r2 as u32]);
                hits += r1 as usize;
            }
            let n = got.len();
            assert_eq!(&x[..n], &got[..], "line {k}: answers");
            let (l, ends, lo, hi) = if ball { (&bq.list, [bq.c, bq.c], bq.lo, bq.hi) } else { (&cq.list, [cq.a, cq.b], cq.lo, cq.hi) };
            let mut st = Vec::new();
            for v in [ends[0], ends[1], l.min, l.max, lo, hi] {
                st.extend(bits(v));
            }
            st.extend([l.done as u32, l.level as u32, l.new as u32, l.new_bits as u32, (l.new_bits >> 32) as u32, l.level_bits as u32, (l.level_bits >> 32) as u32]);
            st.extend([l.high.to_bits(), l.low.to_bits()]);
            assert_eq!(&x[n..n + 27], &st[..], "line {k}: query and list");
            full += (cap > 0 && l.done as usize == cap) as usize;
            for (j, r) in l.records.iter().enumerate() {
                let y = &x[n + 27 + 17 * j..n + 44 + 17 * j];
                let mut e = vec![r.instance];
                for v in [r.point, r.normal, r.bound_normal] {
                    e.extend(bits(v));
                }
                e.extend([r.t.to_bits(), r.depth.to_bits(), r.material, r.index as u32, r.kind as u32, r.below as u32, r.part as u32]);
                assert_eq!(y, &e[..], "line {k}: record {j}");
                recs += (r.instance != 0) as usize;
                if r.instance != 0 {
                    cells_hit.insert(r.part);
                }
            }
        }
        // (sphere cases, dispatches that added records, lists left full, records, cells the records came from)
        assert_eq!((balls, hits, full, recs, cells_hit.len()), (1545, 1908, 1015, 1678, 54), "coverage");
    }

    /// F-89464 / F-83274 / F-12307 run offline (tools/poly_area_reference.py -> testdata/poly_area_ref.txt): random
    /// polygons and every car's hull, as the file has it and moved a little; each polygon's area word equals poly_area.
    #[test]
    fn poly_area_matches_the_original() {
        let h = |s: &str| u32::from_str_radix(s, 16).unwrap();
        let mut n = 0;
        for (k, line) in include_str!("../testdata/poly_area_ref.txt").lines().enumerate() {
            let (inp, out) = line.split_once(" -> ").unwrap();
            let w: Vec<f32> = inp.split(' ').skip(1).map(|s| f32::from_bits(h(s))).collect();
            let o: Vec<u32> = out.split(' ').map(h).collect();
            assert_eq!(w.len(), 9 * o.len(), "line {}", k + 1);
            for (i, &want) in o.iter().enumerate() {
                let v = |j: usize| Vec3::new(w[9 * i + 3 * j], w[9 * i + 3 * j + 1], w[9 * i + 3 * j + 2]);
                assert_eq!(poly_area(v(0), v(1), v(2)).to_bits(), want, "line {} polygon {i}", k + 1);
                n += 1;
            }
        }
        assert_eq!(n, 14_246);
    }

    /// The rail sweep's two level queries against the original's code run whole offline (tools/level_query_reference.py
    /// -> testdata/level_query_ref.txt): F-66549 / F-27454 from F-23371's block, with their query objects, set-ups and
    /// excluded-list callbacks (F-19966 / F-53082), the drivers F-49353 / F-91312 with the culler (F-33380, F-36470 /
    /// F-21683, the level walk F-70638 / F-71552 over one root node, F-62425), the frame steps F-51913 / F-56283, the
    /// dispatches and F-77105 / F-72810; objects the city's grid, prop composites and walker capsules with their states,
    /// type and include flags and matrices. Only the props' material call faked ((index + 1) << 8) and the material
    /// manager's cast answered (shift 8). The sphere's answer, the query's list and every record equal.
    #[test]
    fn level_query_matches_the_original() {
        let Some(city) = city_probe() else { return };
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let Ok(d) = std::fs::read(std::path::Path::new(&root).join("resources/prop/0x0e66973c.r131")) else { return };
        let u32le = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let data = d[16..].to_vec();
        let vs = (u32le(12) as usize).min(data.len());
        let res = mcla_formats::rsc::Resource { rtype: u32le(4), flags: u32le(8), virtual_size: vs, physical_size: data.len() - vs, data };
        let types = mcla_formats::rage::fragphys::parse_frag_types(&res);
        let fake = |_: &mcla_formats::rage::fragphys::FragBound| -> Vec<u32> { (0..256).map(|j| (j + 1) << 8).collect() };
        let props: Vec<ProbeBound> = types.iter().map(|t| ProbeBound::of_prop(t, &fake)).collect();
        let u = |h: &str| u32::from_str_radix(&h.replace('_', ""), 16).unwrap();
        let bits = |x: Vec3| [x.x.to_bits(), x.y.to_bits(), x.z.to_bits()];
        const INST: u32 = 0x40D0_1000;
        let (mut balls, mut blocked, mut kinds, mut excl_hit, mut recs) = (0, 0, [0usize; 3], 0, 0);
        for (k, line) in include_str!("../testdata/level_query_ref.txt").lines().enumerate() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let mut groups = i.split(" | ");
            let h: Vec<u32> = groups.next().unwrap().split_whitespace().map(u).collect();
            let x: Vec<u32> = o.split_whitespace().map(u).collect();
            let f = |i: usize| f32::from_bits(h[i]);
            let ball = h[0] != 0;
            let words: Vec<Vec<u32>> = groups.map(|g| g.split_whitespace().map(u).collect()).collect();
            let walkers: Vec<Option<ProbeBound>> = words
                .iter()
                .map(|w| {
                    (w[0] == 2).then(|| {
                        let (r, l) = (f32::from_bits(w[1]), f32::from_bits(w[2]));
                        let hl = l * 0.5 + r;
                        ProbeBound {
                            kind: 1,
                            radius: hl,
                            centroid: Vec3::ZERO,
                            max: Vec3::new(r, hl, r),
                            min: Vec3::new(-r, -hl, -r),
                            materials: (0..64).map(|j| (j + 1) << 8).collect(),
                            shape: ProbeShape::Prim(Primitive::Capsule { centre: Vec3::ZERO, r, len: l }),
                        }
                    })
                })
                .collect();
            let objs: Vec<LevelObject> = words
                .iter()
                .enumerate()
                .map(|(n, w)| {
                    let p = |i: usize| Vec3::new(f32::from_bits(w[i]), f32::from_bits(w[i + 1]), f32::from_bits(w[i + 2]));
                    kinds[w[0] as usize] += 1;
                    let bound = match w[0] {
                        0 => &city,
                        1 => &props[w[2] as usize],
                        _ => walkers[n].as_ref().unwrap(),
                    };
                    LevelObject { inst: INST + 0x100 * n as u32, state: w[3], arch_type: w[4], arch_include: w[5], m: [p(6), p(9), p(12), p(15)], bound }
                })
                .collect();
            let excluded: Vec<u32> = (0..2).map(|j| if h[13 + j] < 0x100 { INST + 0x100 * h[13 + j] } else { h[13 + j] }).collect();
            let n = (h[12] as i32).clamp(0, 2) as usize;
            let filter = LevelFilter { include: h[10], states: h[11] & 0xFF, excluded: &excluded[..n] };
            let mask = 640 << crate::tcp_rail::MATERIAL_SHIFT;
            let (answer, l) = if ball {
                balls += 1;
                let q = ball_level(Vec3::new(f(3), f(4), f(5)), f(9), mask, 12, [0; 3], &filter, objs.iter().copied());
                (q.list.done as u32, q.list)
            } else {
                (0, capsule_level(Vec3::new(f(3), f(4), f(5)), Vec3::new(f(6), f(7), f(8)), f(9), mask, 1, h[1], &filter, objs.iter().copied()).list)
            };
            let mut st = vec![answer, l.records.len() as u32, l.done as u32, l.level as u32, l.new as u32];
            st.extend([l.new_bits as u32, (l.new_bits >> 32) as u32, l.level_bits as u32, (l.level_bits >> 32) as u32, l.high.to_bits(), l.low.to_bits()]);
            assert_eq!(&x[..11], &st[..], "line {k}: answer and list");
            for (j, r) in l.records.iter().enumerate() {
                let y = &x[11 + 17 * j..28 + 17 * j];
                let mut e = vec![r.instance];
                for v in [r.point, r.normal, r.bound_normal] {
                    e.extend(bits(v));
                }
                e.extend([r.t.to_bits(), r.depth.to_bits(), r.material, r.index as u32, r.kind as u32, r.below as u32, r.part as u32]);
                assert_eq!(y, &e[..], "line {k}: record {j}");
                recs += (r.instance != 0) as usize;
            }
            blocked += (l.records[0].instance != 0) as usize;
            excl_hit += objs.iter().any(|o| excluded[..n].contains(&o.inst)) as usize;
        }
        // (sphere cases, cases blocked, objects by kind - city, props, walkers -, cases excluding one of their objects,
        // records)
        assert_eq!((balls, blocked, kinds, excl_hit, recs), (992, 729, [1174, 2935, 1632], 1295, 953), "coverage");
    }
}
