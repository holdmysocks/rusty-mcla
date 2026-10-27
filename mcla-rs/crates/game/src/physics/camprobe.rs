//! The level as the cameras' probes see it (rules in `docs/reference/cameras/probe_rule.md`): the
//! city and the other cars, a ball's contacts at a point (F-87651 -> F-82650, up to `max` kept by
//! the result list's score, F-89028), a ball swept along a segment (F-43031, directed) and the
//! one-sided line probe (F-20855). The camera's own car is left out by its identity.

use glam::{Mat3, Quat, Vec3};

use super::collision::WorldCollider;
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

fn inside_tri(p: Vec3, v: &[Vec3; 3], n: Vec3) -> bool {
    (0..3).all(|i| n.cross(v[(i + 1) % 3] - v[i]).dot(p - v[i]) >= 0.0)
}

/// A ball moving from `o` by `d` against the capsule of radius `r` round a..b: the first touch
/// (t in 0..1) and the normal there (F-58482 - approx.: the standard first touch, its own
/// arithmetic not read).
fn sweep_capsule(o: Vec3, d: Vec3, a: Vec3, b: Vec3, r: f32) -> Option<(f32, Vec3)> {
    let mut best: Option<(f32, Vec3)> = None;
    let mut take = |t: f32, nrm: Vec3| {
        if (0.0..=1.0).contains(&t) && best.map_or(true, |b| t < b.0) {
            best = Some((t, nrm));
        }
    };
    let ab = b - a;
    let ab2 = ab.length_squared();
    if ab2 > 1e-12 {
        let ao = o - a;
        let dp = d - ab * (d.dot(ab) / ab2);
        let op = ao - ab * (ao.dot(ab) / ab2);
        let qa = dp.length_squared();
        let qb = 2.0 * dp.dot(op);
        let qc = op.length_squared() - r * r;
        let disc = qb * qb - 4.0 * qa * qc;
        if qa > 1e-12 && disc >= 0.0 {
            let t = (-qb - disc.sqrt()) / (2.0 * qa);
            let p = o + d * t;
            let s = (p - a).dot(ab) / ab2;
            if (0.0..=1.0).contains(&s) {
                take(t, (p - (a + ab * s)) / r);
            }
        }
    }
    let dd = d.length_squared();
    if dd > 1e-12 {
        for v in [a, b] {
            let m = o - v;
            let bb = m.dot(d);
            let cc = m.length_squared() - r * r;
            let disc = bb * bb - dd * cc;
            if disc >= 0.0 {
                let t = (-bb - disc.sqrt()) / dd;
                take(t, (o + d * t - v) / r);
            }
        }
    }
    best
}

/// A ball swept from `o` by `d` against one polygon, directed (F-43031): only against its front,
/// from a start more than `r` in front of its plane (nearer, the polygon is left out, edges too);
/// the face at the plane's touch, else the first touch of an edge or corner. Returns (t, normal).
pub fn sweep_polygon(o: Vec3, d: Vec3, r: f32, v: &[Vec3; 3], n: Vec3) -> Option<(f32, Vec3)> {
    let dn = n.dot(d);
    if dn >= 0.0 {
        return None;
    }
    let s = n.dot(o - v[0]);
    if !(s > 0.0) || s - r > dn.abs() {
        return None;
    }
    let t0 = (s - r) / -dn;
    if !(t0 >= 0.0) || t0 > 1.0 {
        return None;
    }
    let ct = o + d * t0;
    if inside_tri(ct, v, n) {
        return Some((t0, n));
    }
    let mut best: Option<(f32, Vec3)> = None;
    for i in 0..3 {
        if let Some((t, nrm)) = sweep_capsule(o, d, v[i], v[(i + 1) % 3], r) {
            if nrm.dot(d) <= 0.0 && best.map_or(true, |b| t < b.0) {
                best = Some((t, nrm));
            }
        }
    }
    best
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
                // (the capsule a..b with r against the car's ball)
                let t = ((o.centre - a).dot(d) / d.length_squared()).clamp(0.0, 1.0);
                if (a + d * t).distance(o.centre) > o.radius + r {
                    continue;
                }
                for (i, v) in o.tris.iter().enumerate() {
                    take(sweep_polygon(a, d, r, v, o.normals[i]));
                }
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
}
