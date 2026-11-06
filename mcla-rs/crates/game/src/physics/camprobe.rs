//! The level as the cameras' probes see it (rules in `docs/reference/cameras/probe_rule.md`): the
//! city and the other cars, a ball's contacts at a point (F-87651 -> F-82650, up to `max` kept by
//! the result list's score, F-89028), a ball swept along a segment (F-43031, directed) and the
//! one-sided line probe (F-20855). The camera's own car is left out by its identity.

use glam::{Mat3, Quat, Vec3};

use super::collision::WorldCollider;
use super::exact::{dot3 as vdot, dot_xyz, fma, recip};
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

/// A ball against one polygon (F-82650): both sides of the plane; the face, else an edge (taken
/// only in front of the plane and only if the neighbour does not own it, F-38934), else a corner.
/// `beside(e)` = the normal of the polygon across edge e.
pub fn ball_polygon(c: Vec3, r: f32, v: &[Vec3; 3], n: Vec3, beside: impl Fn(usize) -> Option<Vec3>) -> Option<Contact> {
    let d = n.dot(c - v[0]);
    if d.abs() > r {
        return None;
    }
    let mut inside = true;
    let mut flag = [0i8; 3];
    let mut corner_search = false;
    for i in 0..3 {
        let (a, b) = (v[i], v[(i + 1) % 3]);
        let e = b - a;
        let (w, u) = (c - a, c - b);
        if n.cross(e).dot(w) >= 0.0 {
            flag[i] = 1;
            continue;
        }
        inside = false;
        if e.dot(w) < 0.0 {
            flag[i] = -1;
            continue;
        }
        if e.dot(u) > 0.0 {
            flag[i] = -2;
            continue;
        }
        if n.dot(w) <= 0.0 {
            continue;
        }
        // the edge: the distance to its line, the point on it
        let (ww, uu, ee) = (w.length_squared(), u.length_squared(), e.length_squared());
        let inv = if ee == 0.0 { 1e8 } else { 1.0 / ee };
        let d2 = (2.0 * (ww + uu) - ee - (ww - uu) * (ww - uu) * inv) * 0.25;
        if d2 > r * r {
            return None;
        }
        let dist = if d2 <= 0.0 { 0.0 } else { d2.sqrt() };
        let s = ((ww - dist * dist) * inv).max(0.0).sqrt();
        let q = a + e * s;
        let normal = if dist <= 1e-12 { n } else { (c - q) / dist };
        // (the neighbour takes the edge when the contact faces it more)
        if beside(i).is_some_and(|nn| normal.dot(nn) > normal.dot(n)) {
            corner_search = true;
            break;
        }
        return Some(Contact { pos: q, normal, depth: r - dist });
    }
    if inside && !corner_search {
        return Some(Contact { pos: c - n * d, normal: n, depth: r - d });
    }
    // the corner between the edges it is outside of
    let i = (0..3).find(|&i| (flag[i] == -2 && flag[(i + 1) % 3] != -2) || (flag[i] != -1 && flag[(i + 1) % 3] == -1))?;
    let p = v[(i + 1) % 3];
    let dv = c - p;
    if dv.length_squared() > r * r {
        return None;
    }
    let dist = dv.length();
    Some(Contact { pos: p, normal: if dist <= 1e-12 { n } else { dv / dist }, depth: r - dist })
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
        self.city.triangles_of_ball(c - ext, c + ext, Some((c, r)), |m, i| {
            let v = &m.tris[i];
            let n = m.faces.get(i).copied().unwrap_or_else(|| (v[1] - v[0]).cross(v[2] - v[0]).normalize_or_zero());
            let beside = |e: usize| m.beside.get(i).and_then(|b| b[e]).map(|j| m.faces.get(j as usize).copied().unwrap_or(Vec3::Z));
            if let Some(k) = ball_polygon(c, r, v, n, beside) {
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
                    if let Some(k) = ball_polygon(c, r, v, o.normals[i], beside) {
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

    const FLOOR: [Vec3; 3] = [Vec3::new(-4.0, 0.0, -4.0), Vec3::new(-4.0, 0.0, 4.0), Vec3::new(4.0, 0.0, -4.0)];

    #[test]
    fn ball_polygon_cases() {
        let n = Vec3::Y;
        // the face, both sides of the plane; behind it the normal is still the face's
        let k = ball_polygon(Vec3::new(-2.0, 0.3, -2.0), 0.5, &FLOOR, n, |_| None).unwrap();
        assert!((k.depth - 0.2).abs() < 1e-6 && k.normal == n && k.pos.y == 0.0);
        let k = ball_polygon(Vec3::new(-2.0, -0.3, -2.0), 0.5, &FLOOR, n, |_| None).unwrap();
        assert!((k.depth - 0.8).abs() < 1e-6 && k.normal == n);
        assert!(ball_polygon(Vec3::new(-2.0, 0.6, -2.0), 0.5, &FLOOR, n, |_| None).is_none());
        // an edge (x = -4) from in front: normal from the edge to the centre
        let k = ball_polygon(Vec3::new(-4.3, 0.3, 0.0), 0.5, &FLOOR, n, |_| None).unwrap();
        assert!((k.depth - (0.5 - 0.18f32.sqrt())).abs() < 1e-5 && (k.pos - Vec3::new(-4.0, 0.0, 0.0)).length() < 1e-5, "{k:?}");
        // the same edge, behind the plane: no edge contact, and outside the face: none
        assert!(ball_polygon(Vec3::new(-4.3, -0.3, 0.0), 0.5, &FLOOR, n, |_| None).is_none());
        // the neighbour across the edge owns it when the contact faces it more
        assert!(ball_polygon(Vec3::new(-4.3, 0.3, 0.0), 0.5, &FLOOR, n, |e| (e == 0).then_some(Vec3::NEG_X)).is_none_or(|k| k.pos == FLOOR[0] || k.pos == FLOOR[1]));
        // a corner
        let k = ball_polygon(Vec3::new(-4.2, 0.1, -4.2), 0.5, &FLOOR, n, |_| None).unwrap();
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
