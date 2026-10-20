//! The car's body against the city's collision.
//!
//! The original gives every vehicle a hull for this - a polyhedron of eighteen points on a
//! car, eleven on a motorcycle, the first part of the composite bound in its fragment file
//! (`rage::bound::find_hull`); its floor is a good hand above the road, so kerbs pass under
//! it and belong to the wheels. Here the hull is tested against the triangles around it by
//! separating axes (the triangle's normal, the hull's faces, the crosses of their edges);
//! the least overlap is the way out and how deep the contact is. Where the hull lies against
//! a triangle's face, every one of its points that is through the face is a contact of its
//! own - a bumper that meets a wall a little askew is held at both ends.
//!
//! The answer follows the original's solver (0x8258A300: collide once a tick, a velocity
//! pass 0x825DE200, then a pass that only moves the body out, 0x82591108): four sweeps over
//! the contacts, each taking the closing speed at its point away and, across it, what
//! friction's cone allows; shallower than 6 mm a contact may still close; the finished
//! impulses are given once more times the contacts' mean elasticity; and whatever stands
//! deeper than 7 mm is moved out, all but its last 7 mm at most.

use super::car::CarState;
use super::collision::{closest_point_tri, Surface, WorldCollider};
use glam::{Mat3, Vec3};

#[derive(Debug, Clone)]
pub struct HullShape {
    pub verts: Vec<Vec3>,
    /// the directions its edges run in, each once
    edge_dirs: Vec<Vec3>,
    /// its faces' outward normals, each plane once
    face_normals: Vec<Vec3>,
    pub min: Vec3,
    pub max: Vec3,
    pub centre: Vec3,
    /// its faces as triangles
    pub tris: Vec<[u16; 3]>,
    /// what each of them is made of (the bound's material numbers: 3 roof, 4 side, 5 undercarriage)
    pub mats: Vec<u32>,
    /// the bound's centre of gravity (+0x50), as the file has it
    pub cg: Vec3,
    /// how many of the corners the detector's scan takes (bound +186; 0: by the older rule - all but the last of 18)
    pub scan: usize,
    /// the file hull's own box (max, min; +0x10 / +0x20, the margin included)
    pub bbox: Option<(Vec3, Vec3)>,
    /// the file composite's wheel parts' places in the car (the model's hubs)
    pub wheel_parts: Vec<Vec3>,
}

impl HullShape {
    /// The corners the detector's scan looks at (0x825E0418: the bound's count, +186).
    pub fn scanned(&self) -> &[Vec3] {
        let n = self.verts.len();
        &self.verts[..if self.scan > 0 { self.scan } else if n == 18 { 17 } else { n }]
    }

    pub fn new(verts: Vec<Vec3>, tris: &[[u16; 3]]) -> HullShape {
        let n = verts.len().max(1) as f32;
        let centre = verts.iter().copied().sum::<Vec3>() / n;
        let (mut min, mut max) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for v in &verts {
            min = min.min(*v);
            max = max.max(*v);
        }
        let mut face_normals: Vec<Vec3> = Vec::new();
        let mut edge_dirs: Vec<Vec3> = Vec::new();
        for t in tris {
            let (a, b, c) = (verts[t[0] as usize], verts[t[1] as usize], verts[t[2] as usize]);
            let mut nrm = (b - a).cross(c - a).normalize_or_zero();
            if nrm == Vec3::ZERO {
                continue;
            }
            if nrm.dot(a - centre) < 0.0 {
                nrm = -nrm;
            }
            if !face_normals.iter().any(|f| f.dot(nrm) > 0.999) {
                face_normals.push(nrm);
            }
            for (p, q) in [(a, b), (b, c), (c, a)] {
                let d = (q - p).normalize_or_zero();
                if d != Vec3::ZERO && !edge_dirs.iter().any(|e| e.dot(d).abs() > 0.999) {
                    edge_dirs.push(d);
                }
            }
        }
        HullShape { verts, edge_dirs, face_normals, min, max, centre, tris: tris.to_vec(), mats: Vec::new(), cg: Vec3::ZERO, scan: 0, bbox: None, wheel_parts: Vec::new() }
    }

    /// The hull's reach along an axis: (least, most).
    fn span(&self, axis: Vec3) -> (f32, f32) {
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        for v in &self.verts {
            let d = v.dot(axis);
            lo = lo.min(d);
            hi = hi.max(d);
        }
        (lo, hi)
    }

    /// The way out of a triangle (given in the hull's own frame, wound with its outside
    /// towards the reader): the axis to move the hull along and how far. None when they do
    /// not touch, or when the hull's middle is behind the triangle.
    fn separate(&self, tri: &[Vec3; 3]) -> Option<(Vec3, f32)> {
        let tn = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or_zero();
        if tn == Vec3::ZERO || tn.dot(self.centre - tri[0]) < 0.0 {
            return None;
        }
        let mid = (tri[0] + tri[1] + tri[2]) / 3.0;
        let mut best: Option<(Vec3, f32, f32)> = None;
        // (false: this axis parts them)
        let mut test = |axis: Vec3, weight: f32, best: &mut Option<(Vec3, f32, f32)>| -> bool {
            // the way out leads to the triangle's outside (or, along the triangle, away from it)
            let along = axis.dot(tn);
            let axis = if along < -1e-3 || (along.abs() <= 1e-3 && axis.dot(self.centre - mid) < 0.0) { -axis } else { axis };
            let (h_lo, h_hi) = self.span(axis);
            let t = [tri[0].dot(axis), tri[1].dot(axis), tri[2].dot(axis)];
            let (t_lo, t_hi) = (t[0].min(t[1]).min(t[2]), t[0].max(t[1]).max(t[2]));
            if h_lo >= t_hi || h_hi <= t_lo {
                return false;
            }
            let overlap = t_hi - h_lo;
            let score = overlap * weight;
            if best.map_or(true, |b| score < b.2) {
                *best = Some((axis, overlap, score));
            }
            true
        };
        if !test(tn, 1.0, &mut best) {
            return None;
        }
        // (the triangle's own face is preferred: a seam between two triangles of one wall must
        // not turn the car aside)
        for f in &self.face_normals {
            if !test(*f, 1.3, &mut best) {
                return None;
            }
        }
        for (p, q) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
            let e = (q - p).normalize_or_zero();
            for d in &self.edge_dirs {
                let a = e.cross(*d);
                if a.length_squared() < 1e-5 {
                    continue;
                }
                if !test(a.normalize(), 1.3, &mut best) {
                    return None;
                }
            }
        }
        best.map(|b| (b.0, b.1))
    }
}

/// A car's hull as the running game has it. The file's hull is reshaped when the car is set
/// up (mcCarSim 0x8232D8C8 sorts its points by where they lie, 0x8232E238 moves them; bikes
/// are left alone): the four lowest points become a floor at BoundClearance - lower by what
/// the car sits lower on its tyres than it was modelled - drawn in so that the flank below
/// the body's widest line leans in at BoundAngle; the points along the sides are set to that
/// widest line on both sides; the two foremost become a bumper bar BoundBumper high and
/// BoundBumperWidth to either side. Checked on three cars of a running game to 3e-8.
pub fn shaped(file: &mcla_formats::rage::bound::Hull, t: &mcla_formats::rage::simtune::SimTune, hub_height: f32, bike: bool) -> HullShape {
    let mut v: Vec<Vec3> = file.verts.iter().map(|p| Vec3::from(*p)).collect();
    if !bike && v.len() >= 8 {
        const EPS: f32 = 0.05;
        let (mut min_y, mut min_x, mut max_x, mut min_z) = (1000.0f32, 0.0f32, 0.0f32, 0.0f32);
        for p in &v {
            min_y = min_y.min(p.y);
            min_x = min_x.min(p.x);
            max_x = max_x.max(p.x);
            min_z = min_z.min(p.z);
        }
        // the floor's ring; the lowest height above it; the widest of the rest
        let (mut y2, mut half) = (1000.0f32, 0.0f32);
        let mut bottom = Vec::new();
        for (i, p) in v.iter().enumerate() {
            if p.y > min_y - EPS && p.y < min_y + EPS {
                bottom.push(i);
            } else if p.y > min_y + EPS {
                y2 = y2.min(p.y);
                if p.x > half {
                    half = p.x;
                } else if p.x < -half {
                    half = -p.x;
                }
            }
        }
        let top = y2 - EPS;
        // how much lower the car sits than its wheels were modelled (0x82363210)
        let f = (t.wheel_front.suspension_offset + t.wheel_back.suspension_offset) * 0.5 + (t.wheel_front.radius + t.wheel_back.radius) * 0.5 - hub_height;
        let drop = (-f).max(0.0);
        // (BoundWidebody is added for a car with the wide body kit: none here)
        let wide = half;
        for p in v.iter_mut() {
            if p.z > min_z - EPS && p.z < min_z + EPS {
                if t.s.bound_bumper > 0.0 {
                    p.x = if p.x > 0.0 { t.s.bound_bumper_width } else if p.x < 0.0 { -t.s.bound_bumper_width } else { 0.0 };
                    p.y = t.s.bound_bumper;
                }
            } else if p.x > min_x - EPS && p.x < min_x + EPS && p.y > y2 + EPS {
                p.x = -wide;
            } else if p.x > max_x - EPS && p.x < max_x + EPS && p.y > y2 + EPS {
                p.x = wide;
            }
        }
        // (the floor last, as the original's loops run)
        let y = (t.s.bound_clearance + drop).min(top);
        let inset = (top - y) * t.s.bound_angle.tan();
        for &i in &bottom {
            let p = &mut v[i];
            p.x = if p.x < 0.0 { inset - wide } else { wide - inset };
            p.y = y;
        }
    }
    let mut shape = HullShape::new(v, &file.tris);
    shape.mats = file.material.clone();
    shape.cg = Vec3::from(file.cg);
    shape.bbox = Some((Vec3::from(file.bbox.0), Vec3::from(file.bbox.1)));
    shape.wheel_parts = file.wheel_parts.iter().map(|p| Vec3::from(*p)).collect();
    shape.scan = if std::env::var_os("MCLA_DBG_OLDSCAN").is_some() { 0 } else { (file.scan as usize).min(shape.verts.len()) };
    shape
}

/// A wheel as the collision has it: a disc as wide as the tyre, its axle along x (a prism of
/// twelve sides stands in for the original's rounded one).
pub fn wheel_shape(radius: f32, half_width: f32) -> HullShape {
    let mut verts = Vec::with_capacity(24);
    for side in [-half_width, half_width] {
        for k in 0..12 {
            let (sin, cos) = (k as f32 * std::f32::consts::TAU / 12.0).sin_cos();
            verts.push(Vec3::new(side, radius * cos, radius * sin));
        }
    }
    let mut tris: Vec<[u16; 3]> = Vec::new();
    for k in 0..12u16 {
        let n = (k + 1) % 12;
        tris.push([k, n, 12 + k]);
        tris.push([n, 12 + n, 12 + k]);
        if k >= 1 && k < 11 {
            tris.push([0, k, k + 1]);
            tris.push([12, 12 + k + 1, 12 + k]);
        }
    }
    HullShape::new(verts, &tris)
}

/// The box that decides which of the ground's triangles a wheel's shape is tried against at
/// all (the original's convex-against-mesh routine 0x825D0C18, its test 0x825CE970): the box
/// around the wheel's shape in the wheel's own axes, grown to take in the same box at the
/// part's "last" place - which for a wheel is never brought up to date and stays where the
/// set-up left it, TravelLimit + 0.1 m above the wheel's place of rest in the car as the car
/// stood a tick ago - and by the mesh's 5 mm on every axis. A triangle that does not touch
/// this box gives no contact however near the tyre is. So flat ground under a level wheel is
/// only found once the tyre touches it, and the more the wheel's box is tipped against the
/// ground - a car's standing pitch of two degrees is enough for a centimetre - the sooner:
/// this, not a rule about depth, is why a wheel that has rolled off a lip has nothing under
/// it for a tick while the road below is a centimetre away.
#[derive(Clone, Copy, Debug)]
pub struct WheelBox {
    /// the box's middle and its axes (the wheel's axle, up and way), with its half sizes
    pub centre: Vec3,
    pub axes: [Vec3; 3],
    pub half: Vec3,
}

impl WheelBox {
    /// `now`: the wheel's centre and axes; `size`: half the shape's width and its radius;
    /// `last`: the centre and axes of the box at the part's last place.
    pub fn new(now: (Vec3, [Vec3; 3]), size: (f32, f32), last: (Vec3, [Vec3; 3])) -> Self {
        let e = Vec3::new(size.0, size.1, size.1);
        let (c, r) = now;
        let d = last.0 - c;
        let q = Vec3::new(d.dot(r[0]), d.dot(r[1]), d.dot(r[2]));
        // (the last box's sizes along the present axes)
        let e_last = Vec3::from_array(std::array::from_fn(|i| (0..3).map(|j| r[i].dot(last.1[j]).abs() * e[j]).sum()));
        let (lo, hi) = ((-e).min(q - e_last), e.max(q + e_last));
        let m = (lo + hi) * 0.5;
        WheelBox { centre: c + r[0] * m.x + r[1] * m.y + r[2] * m.z, axes: r, half: (hi - lo) * 0.5 + Vec3::splat(super::collision::WORLD_MARGIN) }
    }

    /// The four axes the original tries: the triangle's plane and the box's three.
    pub fn touches(&self, tri: &[Vec3; 3], face: Vec3) -> bool {
        let reach = (0..3).map(|i| self.half[i] * face.dot(self.axes[i]).abs()).sum::<f32>();
        if face.dot(tri[0] - self.centre).abs() > reach {
            return false;
        }
        for i in 0..3 {
            let p = tri.map(|v| (v - self.centre).dot(self.axes[i]));
            if p[0].min(p[1]).min(p[2]) > self.half[i] || p[0].max(p[1]).max(p[2]) < -self.half[i] {
                return false;
            }
        }
        true
    }
}

/// Whether a contact from the narrow phase is kept (0x825CEE38, after the detector and
/// before the 2 cm test - it runs on the penetration routine's answers too): for every edge
/// with a neighbour that the contact's point, with its 5 mm, lies within 5.1 mm of, the
/// contact's direction - taken across that edge - must lie no farther from the polygon's
/// normal than the neighbour's does where the two meet in a ridge (measured from between
/// them), and within 2.6 degrees of the polygon's own normal where they do not. `n`: from the
/// polygon to the shape; `across`: each edge's neighbour's normal.
fn edge_keeps(tri: &[Vec3; 3], face: Vec3, across: &[Option<Vec3>; 3], on_b: Vec3, n: Vec3) -> bool {
    for i in 0..3 {
        let Some(other) = across[i] else { continue };
        let (e0, e1) = (tri[i], tri[(i + 1) % 3]);
        if !near_edge(e0, e1, on_b) {
            continue;
        }
        let edge = e1 - e0;
        let ridge = other.cross(edge).dot(face) > 0.0;
        let (b, m) = if ridge { ((face + other).normalize_or(face), other) } else { (face, face) };
        let along = edge.normalize_or_zero();
        let lean = (n - along * n.dot(along)).normalize_or(n);
        if lean.dot(b) < b.dot(m).min(1.0 - 1e-6) - 0.001 {
            return false;
        }
    }
    true
}

/// 0x825CED90: is the point within 5.1 mm of the edge (the mesh's margin times 1.02)?
fn near_edge(e0: Vec3, e1: Vec3, p: Vec3) -> bool {
    let edge = e1 - e0;
    let t = (p - e0).dot(edge);
    let q = if t <= 1e-12 { e0 } else if edge.length_squared() - t <= 1e-12 { e1 } else { e0 + edge * (t / edge.length_squared()) };
    (p - q).length_squared() <= 1.0404 * super::collision::WORLD_MARGIN * super::collision::WORLD_MARGIN
}

/// The contact's normal (0x825D0C18 with 0x825D0600 and 0x825D0950): a cone about the
/// polygon's normal, widened by the normal of every neighbour across an edge the contact's
/// point lies near; the routine's own direction is kept if it is inside that cone, else
/// turned to the cone's nearest rim; with no such edge it is the polygon's normal (unless
/// within 0.8 degrees of it already).
fn cone_normal(tri: &[Vec3; 3], face: Vec3, across: &[Option<Vec3>; 3], on_b: Vec3, n: Vec3) -> Vec3 {
    let (mut axis, mut cos) = (face, 1.0f32);
    let turn = |from: Vec3, towards: Vec3, angle: f32| {
        let about = from.cross(towards);
        if about.length_squared() < 1e-12 { from } else { glam::Quat::from_axis_angle(about.normalize(), angle) * from }
    };
    for i in 0..3 {
        let Some(other) = across[i] else { continue };
        if !near_edge(tri[i], tri[(i + 1) % 3], on_b) {
            continue;
        }
        let d = axis.dot(other);
        if !(cos > d) {
            continue;
        }
        if cos >= 0.9999 {
            let half = (axis + other) * 0.5;
            cos = (0.5 * d + 0.5).max(0.0).sqrt();
            axis = if half.length_squared() > 1e-6 { half.normalize() } else { Vec3::Z };
        } else {
            let (a1, a2) = (d.clamp(-1.0, 1.0).acos(), cos.clamp(-1.0, 1.0).acos());
            axis = turn(axis, other, (a1 - a2) * 0.5);
            cos = ((a1 + a2) * 0.5).cos();
        }
    }
    if cos - 1e-4 > axis.dot(n) {
        if cos >= 0.9999 { axis } else { turn(axis, n, cos.clamp(-1.0, 1.0).acos()) }
    } else {
        n
    }
}

/// What a wheel touches (`wheel_touch`).
#[derive(Clone, Copy, Debug)]
pub struct WheelTouch {
    /// what the wheel is told: the contact's depth times its normal's share of the car's up
    pub depth: f32,
    pub point: Vec3,
    pub normal: Vec3,
    pub surface: Surface,
    /// how the narrow phase came by it (for the debug print): the loop's own distance, or the
    /// penetration routine's
    pub deep: bool,
}

/// One contact a wheel (0x8255D028): of every triangle that touches the wheel shape's box, the
/// original's narrow phase (`gjk::contact`: the nearest-points loop as the original runs it,
/// early stops and all, and its penetration routine) gives one contact if the surfaces are
/// less than 2 cm apart; its normal is kept within what the polygon and its neighbours across
/// the edges the point lies on allow (the polygon's own normal on an open edge); and the
/// deepest of them, depth times the normal's share of the car's `up`, is the wheel's.
pub fn wheel_touch(world: &WorldCollider, core: &super::gjk::WheelCore, up: Vec3, reach_box: Option<&WheelBox>, ball: Option<(Vec3, f32)>) -> Option<WheelTouch> {
    let reach = Vec3::splat(core.round() + CORE + 0.03);
    // (each: corners, normal, surface, and - where the mesh is the city's own bound - what its
    // record says of its neighbours across its three edges: their normals as the original's
    // collision forms them, 0x825D0C18 / 0x82576FF8)
    type Beside = [Option<Vec3>; 3];
    let mut tris: Vec<([Vec3; 3], Vec3, Surface, Option<Beside>)> = Vec::new();
    // (the grid's cells: by the ball of the wheel's box - its own, and the one at the part's last place)
    let ball = ball.filter(|_| !grid_old());
    world.triangles_of_ball(core.centre - reach, core.centre + reach, ball, |mesh, i| {
        let tri = mesh.tris[i];
        let surface = Surface::from_u8(mesh.mats.get(i).copied().unwrap_or(0));
        if mesh.faces.len() == mesh.tris.len() && std::env::var_os("MCLA_DBG_OLDNEIGHBOURS").is_none() {
            tris.push((tri, mesh.faces[i], surface, Some(mesh.beside[i].map(|n| n.map(|j| mesh.faces[j as usize])))));
        } else {
            tris.push((tri, (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or_zero(), surface, None));
        }
    });
    // (a polygon's neighbours are the mesh's own - its record names them - however far their
    // other corner lies: the pool they are looked for in is everything that touches any
    // candidate's own extent, not just what the wheel reaches)
    let pool: Vec<([Vec3; 3], Vec3)> = if tris.iter().all(|t| t.3.is_some()) {
        Vec::new()
    } else if std::env::var_os("MCLA_DBG_NEARPOOL").is_some() || tris.is_empty() {
        tris.iter().map(|t| (t.0, t.1)).collect()
    } else {
        let (mut mn, mut mx) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for (tri, face, _, _) in &tris {
            if *face == Vec3::ZERO || reach_box.map_or(false, |b| !b.touches(tri, *face)) {
                continue;
            }
            for v in tri {
                mn = mn.min(*v);
                mx = mx.max(*v);
            }
        }
        let mut out = Vec::new();
        if mn.x <= mx.x {
            world.triangles_in(mn - Vec3::splat(0.01), mx + Vec3::splat(0.01), |tri, _| out.push((*tri, (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or_zero())));
        }
        out
    };
    let mut best: Option<WheelTouch> = None;
    for (tri, face, surface, named) in tris.iter() {
        let face = *face;
        // (walls too: the test for what a wheel takes is on the contact's normal, below.
        // MCLA_DBG_STEEPWHEEL brings back the older, separate reading of steep faces.)
        let steep_apart = { static D: std::sync::OnceLock<bool> = std::sync::OnceLock::new(); *D.get_or_init(|| std::env::var_os("MCLA_DBG_STEEPWHEEL").is_some()) };
        if face == Vec3::ZERO || (steep_apart && face.dot(up) <= 0.05) {
            continue;
        }
        if reach_box.map_or(false, |b| !b.touches(tri, face)) {
            continue;
        }
        // the neighbours across its edges: where the two meet in a ridge, the other's normal
        let shared = |o: &[Vec3; 3], v: Vec3| o.iter().any(|w| (*w - v).length_squared() < 1e-6);
        // (each: the neighbour's normal, and whether the two meet in a ridge)
        let across: [Option<(Vec3, bool)>; 3] = match named {
            Some(beside) => std::array::from_fn(|i| beside[i].map(|other| (other, super::gjk::edge_bisector(face, other, tri[i], tri[(i + 1) % 3]).1))),
            None => std::array::from_fn(|i| {
                let (e0, e1) = (tri[i], tri[(i + 1) % 3]);
                let other = pool.iter().find(|o| shared(&o.0, e0) && shared(&o.0, e1) && !tri.iter().all(|v| shared(&o.0, *v)))?;
                let far = other.0.iter().copied().find(|w| (*w - e0).length_squared() >= 1e-6 && (*w - e1).length_squared() >= 1e-6).unwrap_or(e0);
                Some((other.1, face.dot(far - e0) < -1e-4))
            }),
        };
        let bisectors: [Vec3; 3] = match named {
            Some(beside) => std::array::from_fn(|i| beside[i].map_or(face, |other| super::gjk::edge_bisector(face, other, tri[i], tri[(i + 1) % 3]).0)),
            None => across.map(|n| match n {
                Some((other, true)) => (face + other).normalize_or(face),
                _ => face,
            }),
        };
        let Some(r) = super::gjk::contact(core, tri, face, bisectors, super::gjk::WHEEL_MARGIN, super::collision::WORLD_MARGIN) else { continue };
        // (the sink, 0x825C65E8)
        if r.distance >= 0.02 {
            continue;
        }
        let beside = across.map(|n| n.map(|n| n.0));
        let dbg = { static D: std::sync::OnceLock<bool> = std::sync::OnceLock::new(); *D.get_or_init(|| std::env::var_os("MCLA_DBG_WTOUCH").is_some()) };
        if dbg {
            // (to the last digit, for laying beside a recording's contacts)
            let kept = cone_normal(tri, face, &beside, r.on_b, r.normal);
            eprintln!("WG {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {} {:?} {:?} {:?}", r.normal.x, r.normal.y, r.normal.z, r.on_a.x, r.on_a.y, r.on_a.z, r.on_b.x, r.on_b.y, r.on_b.z, -r.distance, r.deep as u8, kept.x, kept.y, kept.z);
            eprintln!("WT face {:.3?} gjk n {:.3?} dist {:.5} deep {} degenerate {} on_b {:.3?} beside {:.3?} near {:?} keeps {} cone {:.3?}", face, r.normal, r.distance, r.deep, r.degenerate, r.on_b, beside, [0, 1, 2].map(|i| near_edge(tri[i], tri[(i + 1) % 3], r.on_b)), edge_keeps(tri, face, &beside, r.on_b, r.normal), cone_normal(tri, face, &beside, r.on_b, r.normal));
        }
        let exact = named.is_some() && !old_cone();
        if exact {
            let about = [Vec3::ZERO, tri[1] - tri[0], tri[2] - tri[0]];
            let edge: [(Vec3, Vec3); 3] = std::array::from_fn(|e| match across[e] {
                Some((nn, ridge)) => (bisectors[e], if ridge { nn } else { face }),
                None => (face, face),
            });
            if edge_test(&about, across.map(|a| a.is_some()), &edge, r.on_b - tri[0], r.normal).is_some() {
                continue;
            }
        } else if std::env::var_os("MCLA_DBG_NOEDGETEST").is_none() && !edge_keeps(tri, face, &beside, r.on_b, r.normal) {
            continue;
        }
        let normal = if exact { cone_exact(tri, face, &beside, r.on_b, r.normal) } else { cone_normal(tri, face, &beside, r.on_b, r.normal) };
        // (0x8255D028: only what faces up, for the car - as it stood at its last update -
        // and for the world. On an upright wall the second test is on the sign of a rounding
        // error, in the original as here.)
        if normal.dot(up) < 0.0 || normal.y < 0.0 {
            continue;
        }
        let depth = -r.distance * normal.dot(up);
        if best.map_or(true, |b| depth > b.depth) {
            // (0x8255D028: the record's point is the contact's point on the wheel moved out
            // along the normal by that depth - not the point on the polygon)
            let point = if std::env::var_os("MCLA_DBG_OLDTYREIN").is_some() { r.on_b } else { normal * depth + if std::env::var_os("MCLA_DBG_PB").is_some() { r.on_b } else { r.on_a } };
            best = Some(WheelTouch { depth, point, normal, surface: *surface, deep: r.deep });
        }
    }
    if std::env::var_os("MCLA_DBG_WTOUCH").is_some() {
        eprintln!("WT == best {:?}", best.map(|b| (b.depth, b.normal)));
    }
    best
}

/// A wheel shape's margin and the city mesh's together: what its core stands back from a contact.
const CORE: f32 = 0.04 + super::collision::WORLD_MARGIN;

/// Where a wheel's round comes to rest on the ground: a circle of `radius` in the plane
/// through `origin` across `axle`, its centre let down from `origin` along `-up`, and the
/// first thing it touches - the face of a triangle or the end of one where the ground has an
/// edge (a wheel leaving a ledge still rests on its lip; one meeting a step is on its corner
/// before the axle is over it). The answer is given as a ray under the axle would measure it:
/// `t` = the centre's way down + the radius. Only what lies at least `lowest` below `origin`
/// is ground to a wheel, and nothing farther than `reach`.
///
/// Two answers: on the surface the wheel stood on (`kept`: its normal and n.p), and on
/// everything else.
///
/// `reach_box`: the box a triangle has to touch to be looked at at all (`WheelBox`).
#[allow(clippy::too_many_arguments)]
pub fn wheel_rest(world: &WorldCollider, origin: Vec3, up: Vec3, along: Vec3, axle: Vec3, radius: f32, reach: f32, lowest: f32, centre: f32, kept: Option<(Vec3, f32)>, reach_box: Option<&WheelBox>) -> [Option<super::collision::Hit>; 2] {
    let a = origin - along * radius - up * reach;
    let b = origin + along * radius;
    let (mn, mx) = (a.min(b) - Vec3::splat(0.01), a.max(b) + Vec3::splat(0.01));
    // (all of them first: an edge's neighbour decides what a contact on that edge may be)
    let mut tris: Vec<([Vec3; 3], Vec3, Surface)> = Vec::new();
    world.triangles_in(mn, mx, |tri, surface| tris.push((*tri, (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or_zero(), surface)));
    let here = origin - up * centre;
    // (the best so far on each: the centre's way down, how flat the face, the answer)
    let mut best: [Option<(f32, f32, super::collision::Hit)>; 2] = [None, None];
    for (index, (tri, face, surface)) in tris.iter().enumerate() {
        let (face, surface) = (*face, *surface);
        let flat = face.dot(up);
        if flat <= 0.05 {
            continue;
        }
        let dbg = { static D: std::sync::OnceLock<bool> = std::sync::OnceLock::new(); *D.get_or_init(|| std::env::var_os("MCLA_DBG_REST").is_some()) };
        if dbg {
            let dd = [(tri[0] - origin).dot(axle), (tri[1] - origin).dot(axle), (tri[2] - origin).dot(axle)];
            let pl = face.dot(here - tri[0]);
            eprintln!("REST tri {:?} flat {:.4} box {} axle {:?} plane dist of centre {:.4} radius {:.4} v {:?}", surface, flat, reach_box.map_or(true, |b| b.touches(tri, face)), dd, pl, radius, tri.map(|q| ((q - origin).dot(along), (q - origin).dot(up))));
        }
        if reach_box.map_or(false, |b| !b.touches(tri, face)) {
            continue;
        }
        // the triangle's cut through the wheel's plane (and which edge each end lies on)
        let d = [(tri[0] - origin).dot(axle), (tri[1] - origin).dot(axle), (tri[2] - origin).dot(axle)];
        let mut ends = [(Vec3::ZERO, 0usize); 2];
        let mut n = 0;
        for i in 0..3 {
            let j = (i + 1) % 3;
            if (d[i] <= 0.0) != (d[j] <= 0.0) && n < 2 {
                ends[n] = (tri[i] + (tri[j] - tri[i]) * (d[i] / (d[i] - d[j])), i);
                n += 1;
            }
        }
        if n < 2 {
            continue;
        }
        // in the plane: u along the wheel's way, v upwards, the centre at (0, -s)
        let p = ends.map(|(q, _)| ((q - origin).dot(along), (q - origin).dot(up)));
        let mut take = |u: f32, v: f32, s: f32, normal: Vec3, low: f32| {
            if dbg { eprintln!("REST   take u {u:.4} v {v:.4} s {s:.4} depth {:.4} n.up {:.4} low {low:.4} reach ok {}", centre - s, normal.dot(up), s + radius <= reach); }
            if !(s.is_finite() && s + radius <= reach && -v > low) {
                return;
            }
            let point = origin + along * u + up * v;
            // (the same surface: its plane within 5 mm here and all but parallel)
            let which = match kept {
                Some((n, d)) if face.dot(n) > 0.9987 && (n.dot(point) - d).abs() < 0.005 => 0,
                _ => 1,
            };
            if best[which].as_ref().map_or(true, |b| s < b.0 - 1e-5 || (s < b.0 + 1e-5 && flat > b.1)) {
                best[which] = Some((s, flat, super::collision::Hit { t: s + radius, point, normal, surface }));
            }
        };
        // (the end of a triangle: the original's contact there is the polygon's nearest point
        // and its depth the distance to the wheel's round as the wheel stands now - `centre`
        // below `origin` - not what the wheel would have to drop to sit on it: recorded on a
        // lip 14 cm behind the axle, -0.0174 where the drop is -0.0204. Its normal (0x825CEE38,
        // 0x825D0600): on an edge no other polygon shares, the polygon's own; on one that is
        // shared, the way from the edge to the wheel - if that lies between the two polygons'
        // normals, which it only can where they meet in a ridge (a kerb's corner); otherwise
        // the edge gives no contact. And what the wheel is told is the depth's share along
        // the car's up (0x8255D028).)
        for ((u, v), (q, i)) in p.into_iter().zip(ends) {
            let mut through = radius - (u * u + (v + centre) * (v + centre)).sqrt();
            // (where the shape's core - it without its 4 cm margin, the mesh's 5 mm off too -
            // reaches the polygon itself, the original measures as it does for a hull
            // (0x825E2370): the shape backed out along the polygon's normal until its core is
            // clear by the margins and a centimetre, the nearest points found there, the depth
            // what was backed out along their line less their distance. Recorded on a kerb met
            // at 25 m/s, the axle 11 cm short of its edge: 0.1389, not the 0.132 the round is
            // through that edge.)
            let mut from = here;
            if through > CORE {
                let off = (radius - CORE) - face.dot(here - tri[0]) + CORE + 0.01;
                let (cu, cv) = (face.dot(along) * off - u, face.dot(up) * off - centre - v);
                let dist = (cu * cu + cv * cv).sqrt();
                if off > 0.0 && dist > 1e-6 {
                    through = off * (face.dot(along) * cu + face.dot(up) * cv) / dist - (dist - radius);
                    from = here + face * off;
                }
            }
            if dbg { eprintln!("REST   end u {u:.4} v {v:.4} through {through:.4}"); }
            if through <= -0.05 {
                continue;
            }
            let (e0, e1) = (tri[i], tri[(i + 1) % 3]);
            let shared = |o: &[Vec3; 3], v: Vec3| o.iter().any(|w| (*w - v).length_squared() < 1e-6);
            let neighbour = tris.iter().enumerate().find(|(k, o)| *k != index && shared(&o.0, e0) && shared(&o.0, e1));
            let (normal, low) = match neighbour {
                None => (face, lowest),
                Some((_, other)) => {
                    let far = other.0.iter().copied().find(|w| (*w - e0).length_squared() >= 1e-6 && (*w - e1).length_squared() >= 1e-6).unwrap_or(e0);
                    // (a ridge: the neighbour falls away behind this polygon's plane)
                    if face.dot(far - e0) > -1e-4 {
                        continue;
                    }
                    let edge = (e1 - e0).normalize_or_zero();
                    let to_wheel = (from - q).normalize_or_zero();
                    let across = (to_wheel - edge * to_wheel.dot(edge)).normalize_or_zero();
                    let middle = (face + other.1).normalize_or_zero();
                    if across.dot(middle) < middle.dot(other.1) - 1e-4 {
                        continue;
                    }
                    (across, centre)
                }
            };
            take(u, v, centre - through * normal.dot(up).max(0.0), normal, low);
        }
        let (eu, ev) = (p[1].0 - p[0].0, p[1].1 - p[0].1);
        let len = (eu * eu + ev * ev).sqrt();
        if len > 1e-6 && eu.abs() > 1e-6 {
            let (eu, ev) = (eu / len, ev / len);
            // (the cut's normal, the side that looks up)
            let (nu, nv) = if eu > 0.0 { (-ev, eu) } else { (ev, -eu) };
            let along_cut = (-nu * radius - p[0].0) / eu;
            if (0.0..=len).contains(&along_cut) {
                let (u, v) = (p[0].0 + eu * along_cut, p[0].1 + ev * along_cut);
                take(u, v, -(v + nv * radius), face, lowest);
            }
        }
    }
    best.map(|b| b.map(|b| b.2))
}

/// The steep face a shape stands deepest in: the walls, kerb faces and posts around a wheel
/// (what lies flatter than `flat` against `up` is ground, and measured otherwise).
pub fn deepest_steep(hull: &HullShape, pos: Vec3, rm: Mat3, world: &WorldCollider, up: Vec3, flat: f32) -> Option<BodyContact> {
    let (mut mn, mut mx) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for v in &hull.verts {
        let w = pos + rm * *v;
        mn = mn.min(w);
        mx = mx.max(w);
    }
    let inv = rm.transpose();
    let mut best: Option<BodyContact> = None;
    world.triangles_in(mn, mx, |tri, surface| {
        let face = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or_zero();
        if face.dot(up) > flat {
            return;
        }
        let local = [inv * (tri[0] - pos), inv * (tri[1] - pos), inv * (tri[2] - pos)];
        let Some((axis, depth)) = hull.separate(&local) else { return };
        let normal = rm * axis;
        if normal.dot(up) > flat || best.map_or(false, |b| b.depth >= depth) {
            return;
        }
        let (lo, _) = hull.span(axis);
        let (mut sum, mut count) = (Vec3::ZERO, 0.0);
        for v in &hull.verts {
            if v.dot(axis) < lo + 0.02 {
                sum += *v;
                count += 1.0;
            }
        }
        let at = closest_point_tri(sum / count, &local);
        best = Some(BodyContact { point: pos + rm * at, normal, depth, surface, face: normal });
    });
    best
}

#[derive(Debug, Clone, Copy)]
pub struct BodyContact {
    pub point: Vec3,
    /// the way the body is pushed
    pub normal: Vec3,
    pub depth: f32,
    pub surface: Surface,
    /// the polygon's own normal
    pub face: Vec3,
}

/// What the solver keeps of a contact from one tick to the next (the manifold's cache): the
/// body's point of it in the body's frame, and the multipliers of the velocity and the push
/// solve it ended with - where the next tick's solve of the same contact starts.
pub type Kept = (Vec3, Vec3, Vec3);

/// Everything the hull touches, in the world's frame.
pub fn contacts(hull: &HullShape, pos: Vec3, rm: Mat3, world: &WorldCollider, out: &mut Vec<BodyContact>) {
    // the hull's box in the world, with its margin and the 2 cm a contact may stand off
    let (mut mn, mut mx) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for i in 0..8 {
        let c = Vec3::new(if i & 1 == 0 { hull.min.x } else { hull.max.x }, if i & 2 == 0 { hull.min.y } else { hull.max.y }, if i & 4 == 0 { hull.min.z } else { hull.max.z });
        let w = pos + rm * c;
        mn = mn.min(w);
        mx = mx.max(w);
    }
    let reach = Vec3::splat(HULL_MARGIN + 0.02);
    let inv = rm.transpose();
    // (all of them first: a contact on an edge two triangles share takes its normal from both)
    let mut tris: Vec<([Vec3; 3], Vec3, Surface)> = Vec::new();
    world.triangles_in(mn - reach, mx + reach, |tri, surface| {
        let local = [inv * (tri[0] - pos), inv * (tri[1] - pos), inv * (tri[2] - pos)];
        tris.push((local, (local[1] - local[0]).cross(local[2] - local[0]).normalize_or_zero(), surface));
    });
    // The original's narrow phase for a convex part against one polygon (0x825CEE38): GJK
    // between the hull's core and the bare triangle, the margins (4 cm and 5 mm) taken off
    // afterwards. Where the cores overlap (0x825E2370) the hull is first backed out along the
    // polygon's normal until its deepest point is clear by the margins and a centimetre, the
    // nearest points are found there, and the depth is what was backed out along their line
    // less their distance. ONE contact a polygon: on the triangle, at the point nearest the
    // part of the hull that leads - an end of the bumper bar nose-first into a wall, or where
    // the bar crosses on to that triangle. (Recorded: an R8 at 26 m/s, two contacts, both at the
    // right end of its bar, 0.33 deep; an S281, three, at 0.36 and 0.55 along its bar where two
    // facets of the same wall meet, 0.37 deep.)
    for (index, (tri, face, surface)) in tris.iter().enumerate() {
        let face = *face;
        if face == Vec3::ZERO {
            continue;
        }
        // (the literal routine. What keeps a wall's other polygons out - an R8's bar through
        // three facets gives two contacts, an S281's three - is the edge test, `edge_keeps`.)
        let old = { static D: std::sync::OnceLock<bool> = std::sync::OnceLock::new(); *D.get_or_init(|| std::env::var_os("MCLA_DBG_OLDHULL").is_some()) };
        let mut exact: Option<Vec3> = None;
        let (on_hull, on_tri, through) = if !old {
            // (the routine itself, in the world's axes as the original runs it: the hull's
            // core is its corners, its margin 4 cm; the directions it may be backed out along
            // are the polygon's normal and what lies between that and its neighbours' across
            // the edges where they meet in a ridge)
            let shared = |o: &[Vec3; 3], v: Vec3| o.iter().any(|w| (*w - v).length_squared() < 1e-6);
            let bisectors: [Vec3; 3] = std::array::from_fn(|i| {
                let (e0, e1) = (tri[i], tri[(i + 1) % 3]);
                match tris.iter().enumerate().find(|(k, o)| *k != index && shared(&o.0, e0) && shared(&o.0, e1)) {
                    Some((_, other)) => {
                        let far = other.0.iter().copied().find(|w| (*w - e0).length_squared() >= 1e-6 && (*w - e1).length_squared() >= 1e-6).unwrap_or(e0);
                        if face.dot(far - e0) < -1e-4 { rm * (face + other.1).normalize_or(face) } else { rm * face }
                    }
                    None => rm * face,
                }
            });
            // (the core is the hull's corners but its last, which the original's count leaves
            // out; the search for the farthest begins at the 17th)
            let used = if hull.verts.len() == 18 { 17 } else { hull.verts.len() };
            let corners: Vec<Vec3> = (0..used).map(|k| (k + used - 1) % used).map(|k| pos + rm * hull.verts[k]).collect();
            let there = [pos + rm * tri[0], pos + rm * tri[1], pos + rm * tri[2]];
            let Some(r) = super::gjk::contact(&super::gjk::Points(&corners), &there, rm * face, bisectors, 0.04, super::collision::WORLD_MARGIN) else { continue };
            let line = inv * r.normal;
            if std::env::var_os("MCLA_DBG_HIT").is_some() {
                eprintln!("HULLTRI {index} face {:.3?} verts {:.2?} depth {:.4} deep {} line {:.3?} on hull {:.3?} area {:.3}", face, tri, -r.distance, r.deep, line, inv * (r.on_a - pos), (tri[1] - tri[0]).cross(tri[2] - tri[0]).length() * 0.5);
            }
            let beside: [Option<Vec3>; 3] = std::array::from_fn(|i| {
                let (e0, e1) = (tri[i], tri[(i + 1) % 3]);
                tris.iter().enumerate().find(|(k, o)| *k != index && shared(&o.0, e0) && shared(&o.0, e1)).map(|(_, o)| o.1)
            });
            let at = inv * (r.on_b - pos);
            if std::env::var_os("MCLA_DBG_NOEDGETEST").is_none() && !edge_keeps(tri, face, &beside, at, line) {
                continue;
            }
            exact = Some(cone_normal(tri, face, &beside, at, line));
            (inv * (r.on_a - pos), at - line * super::collision::WORLD_MARGIN, (-r.distance, line))
        } else {
            let deepest = hull.verts.iter().map(|v| -face.dot(*v - tri[0])).fold(f32::NEG_INFINITY, f32::max);
            if hull.separate(tri).is_some() {
                let off = deepest + HULL_MARGIN + 0.01;
                let (a, b) = hull.nearest(tri, face * off);
                let gap = a - b;
                let dist = gap.length();
                // (and only if, backed out so, the hull is then no farther from THIS triangle than
                // the 2 cm a contact may stand off: a triangle the hull's leading point is not over
                // gives none, however deep the rest of the nose is through its plane. Read off the
                // two recorded hits - the R8's bar runs on over a third facet of the wall, 22 cm
                // deep, with no contact there; the code's own test was not found)
                if dist > HULL_MARGIN + 0.01 + 0.02 {
                    continue;
                }
                let line = if dist > 1e-6 { gap / dist } else { face };
                (a - face * off, b, (off * face.dot(line) - (dist - HULL_MARGIN), line))
            } else {
                let (a, b) = hull.nearest(tri, Vec3::ZERO);
                let gap = a - b;
                let dist = gap.length();
                if dist < 1e-6 {
                    continue;
                }
                (a, b, (HULL_MARGIN - dist, gap / dist))
            }
        };
        let (depth, line) = through;
        // (the sink, 0x825C65E8: nothing farther than 2 cm)
        if depth <= -0.02 {
            continue;
        }
        let _ = on_hull;
        // the normal (0x825D0600): the nearest points' line kept within what the polygon and
        // the neighbours across the edges the point lies on allow - the polygon's own normal
        // where it has no such neighbour
        let mut normal = face;
        for i in 0..3 {
            let (e0, e1) = (tri[i], tri[(i + 1) % 3]);
            let along = (e1 - e0).normalize_or_zero();
            let from = on_tri - e0;
            if (from - along * from.dot(along)).length() > 0.0051 {
                continue;
            }
            let shared = |o: &[Vec3; 3], v: Vec3| o.iter().any(|w| (*w - v).length_squared() < 1e-6);
            if let Some((_, other)) = tris.iter().enumerate().find(|(k, o)| *k != index && shared(&o.0, e0) && shared(&o.0, e1)) {
                let across = (line - along * line.dot(along)).normalize_or_zero();
                let middle = (face + other.1).normalize_or_zero();
                normal = if across.dot(middle) >= middle.dot(other.1) - 1e-3 { across } else if across.dot(other.1) > across.dot(face) { other.1 } else { face };
            }
        }
        let normal = exact.unwrap_or(normal);
        if normal == Vec3::ZERO {
            continue;
        }
        out.push(BodyContact { point: pos + rm * on_tri, normal: rm * normal, depth, surface: *surface, face: rm * face });
    }
}

/// The nearest points of two triangles (the first's, the second's).
fn nearest_tri_tri(a: &[Vec3; 3], b: &[Vec3; 3]) -> (Vec3, Vec3) {
    let mut best = (a[0], b[0], f32::INFINITY);
    let mut take = |p: Vec3, q: Vec3| {
        let d = (p - q).length_squared();
        if d < best.2 {
            best = (p, q, d);
        }
    };
    for v in a {
        take(*v, closest_point_tri(*v, b));
    }
    for v in b {
        take(closest_point_tri(*v, a), *v);
    }
    for i in 0..3 {
        for j in 0..3 {
            let (p0, p1, q0, q1) = (a[i], a[(i + 1) % 3], b[j], b[(j + 1) % 3]);
            // nearest points of two segments
            let (d1, d2, r) = (p1 - p0, q1 - q0, p0 - q0);
            let (aa, ee, ff) = (d1.length_squared(), d2.length_squared(), d2.dot(r));
            if aa < 1e-12 || ee < 1e-12 {
                continue;
            }
            let (cc, bb) = (d1.dot(r), d1.dot(d2));
            let denom = aa * ee - bb * bb;
            let mut s = if denom > 1e-12 { ((bb * ff - cc * ee) / denom).clamp(0.0, 1.0) } else { 0.0 };
            let mut t = (bb * s + ff) / ee;
            if t < 0.0 {
                t = 0.0;
                s = (-cc / aa).clamp(0.0, 1.0);
            } else if t > 1.0 {
                t = 1.0;
                s = ((bb - cc) / aa).clamp(0.0, 1.0);
            }
            take(p0 + d1 * s, q0 + d2 * t);
        }
    }
    (best.0, best.1)
}

impl HullShape {
    /// The nearest points of the hull, moved by `shift`, and a triangle it does not reach
    /// into: (on the moved hull, on the triangle).
    pub fn nearest(&self, tri: &[Vec3; 3], shift: Vec3) -> (Vec3, Vec3) {
        let mut best = (self.verts[0] + shift, tri[0], f32::INFINITY);
        for t in &self.tris {
            let face = [self.verts[t[0] as usize] + shift, self.verts[t[1] as usize] + shift, self.verts[t[2] as usize] + shift];
            let (p, q) = nearest_tri_tri(&face, tri);
            let d = (p - q).length_squared();
            if d < best.2 {
                best = (p, q, d);
            }
        }
        (best.0, best.1)
    }
}

/// The hull's margin and the city mesh's (bound +0x70 of the car's polyhedron: 0.04; 0.005).
pub const HULL_MARGIN: f32 = 0.04 + super::collision::WORLD_MARGIN;

/// What a contact is made of: (friction, elasticity).
pub type Material = (f32, f32);

/// One contact as the solver has it: its three directions, where it is from the body's place,
/// the inverse of what an impulse there does to the point's velocity, and its friction.
struct Row {
    basis: [Vec3; 3],
    r: Vec3,
    inverse: Mat3,
    /// the first row of the block over its first entry: what the normal impulse would be
    /// with no friction
    c: Vec3,
    mu: f32,
}

impl Row {
    fn new(n: Vec3, r: Vec3, inv_m: f32, inv_i: Mat3, mu: f32) -> Row {
        // (0x821CE640)
        let b1 = if n.x.abs() >= n.y.abs() { Vec3::new(-n.z, 0.0, n.x) } else { Vec3::new(0.0, n.z, -n.y) }.normalize_or(Vec3::X);
        let basis = [n, b1, n.cross(b1)];
        // (0x825E4C40: K[i][j] = basis[i] . (basis[j] / m + (I^-1 (r x basis[j])) x r))
        let cols: [Vec3; 3] = std::array::from_fn(|j| {
            let moved = basis[j] * inv_m + (inv_i * r.cross(basis[j])).cross(r);
            Vec3::new(basis[0].dot(moved), basis[1].dot(moved), basis[2].dot(moved))
        });
        let k = Mat3::from_cols(cols[0], cols[1], cols[2]);
        let k00 = k.x_axis.x;
        let c = if k00 == 0.0 { Vec3::X } else { Vec3::new(1.0, k.y_axis.x / k00, k.z_axis.x / k00) };
        Row { basis, r, inverse: if k.determinant().abs() > 1e-20 { k.inverse() } else { Mat3::ZERO }, c, mu }
    }

    fn world(&self, p: Vec3) -> Vec3 {
        self.basis[0] * p.x + self.basis[1] * p.y + self.basis[2] * p.z
    }
}

/// Four Gauss-Seidel sweeps of the kernel over the rows from the velocities given, each row
/// wanting its point's velocity to be `bias` less than nothing along (normal, two across).
/// Returns what the rows together add to the body's velocity and turning.
fn sweep(rows: &[Row], bias: &[Vec3], vel: Vec3, ang: Vec3, inv_m: f32, inv_i: Mat3, omega: f32, given: &mut [Vec3]) -> (Vec3, Vec3) {
    let (mut v, mut w) = (vel, ang);
    // (0x825E8B90: what the rows start with - last tick's answers - is applied first)
    for (row, p) in rows.iter().zip(given.iter()) {
        let push = row.world(*p);
        v += push * inv_m;
        w += inv_i * row.r.cross(push);
    }
    for _ in 0..4 {
        let mut residual = 0.0;
        for (i, row) in rows.iter().enumerate() {
            let at = v + w.cross(row.r);
            let u = Vec3::new(row.basis[0].dot(at), row.basis[1].dot(at), row.basis[2].dot(at)) + bias[i];
            let x = given[i] - row.inverse * u * omega;
            let across = (x.y * x.y + x.z * x.z).sqrt();
            let nn = row.c.dot(x);
            let next = if nn < 0.0 {
                Vec3::ZERO
            } else if across > row.mu * x.x {
                let d = x - Vec3::new(nn, 0.0, 0.0);
                let g = Vec3::new(-row.mu * across, x.y, x.z);
                x - d * (across * (across - row.mu * x.x) / (d.dot(g) + 1e-8))
            } else {
                x
            };
            let step = next - given[i];
            given[i] = next;
            residual += step.length_squared();
            let push = row.world(step);
            v += push * inv_m;
            w += inv_i * row.r.cross(push);
        }
        if residual == 0.0 {
            break;
        }
    }
    (v - vel, w - ang)
}

thread_local! {
    /// whether the last `resolve` on this thread found the hull in contact with anything
    pub static TOUCHED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// how many of those contacts stood all but upright (their normal's height under 0.1) and
    /// were answered with an impulse
    pub static WALLS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// whether that `resolve` changed the body's velocities at all
    pub static HIT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Answers the hull's contacts, once a tick: impulses on the body (its mass and its moments
/// of inertia about its own axes, turning about `com`), then the way out. Returns the hardest
/// closing speed met and where.
#[allow(clippy::too_many_arguments)]
pub fn resolve(st: &mut CarState, hull: &HullShape, world: &WorldCollider, mass: f32, inertia: Vec3, com_local: Vec3, dt: f32, material: impl Fn(Surface, Vec3, f32) -> Material, head_on: Option<(f32, f32, f32)>, kept: &mut Vec<Kept>, pending: (Vec3, Vec3)) -> Option<(f32, Vec3)> {
    let rm = Mat3::from_quat(st.rot);
    let mut found: Vec<BodyContact> = Vec::new();
    contacts(hull, st.pos, rm, world, &mut found);
    // (0x825CEE38, before its edge test: a contact whose point on the body has moved out
    // along the polygon's normal since the last tick is none)
    if std::env::var_os("MCLA_DBG_NOAWAYTEST").is_none() {
        let centre = st.pos + rm * com_local;
        found.retain(|c| c.face.dot(st.vel + st.ang_vel.cross(c.point - c.normal * c.depth - centre)) * dt <= 1e-6);
    }
    TOUCHED.with(|t| t.set(!found.is_empty()));
    WALLS.with(|t| t.set(0));
    HIT.with(|t| t.set(false));
    if found.is_empty() {
        kept.clear();
        return None;
    }
    let before = (st.vel, st.ang_vel, st.pos);
    // (the deepest first: in four sweeps the first contact to be asked takes most of a blow,
    // and a point that barely touches, high on the nose, must not be the one - it tipped an
    // R8 nose-up off a wall where the recorded car's tail comes up)
    if std::env::var_os("MCLA_DBG_NOSORT").is_none() {
        found.sort_by(|a, b| b.depth.partial_cmp(&a.depth).unwrap_or(std::cmp::Ordering::Equal));
    }
    let com = st.pos + rm * com_local;
    static DBG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *DBG.get_or_init(|| std::env::var("MCLA_DBG_HIT").is_ok()) {
        for c in &found {
            let v = st.vel + st.ang_vel.cross(c.point - com);
            eprintln!("HIT at local {:.2?} normal local {:.2?} depth {:.3} {:?} closing {:.2}", rm.transpose() * (c.point - st.pos), rm.transpose() * c.normal, c.depth, c.surface, -v.dot(c.normal));
        }
        eprintln!("HIT --");
    }
    let inv_i = rm * Mat3::from_diagonal(Vec3::ONE / inertia.max(Vec3::splat(1.0))) * rm.transpose();
    let inv_m = 1.0 / mass.max(1.0);
    // (a head-on hit, 0x82332D30: faster than 22.35 m/s, against what faces the car's nose
    // within 25 degrees, the contact is the tuning's BoundFriction and BoundElasticity instead
    // of the materials', and it is taken HeadOnCGOffset lower than it is - which is what
    // brings a car's tail up off a wall)
    let back = rm.z_axis;
    // (which of the hull's own materials: by how high on it the contact is, 0 its floor .. 1 its top)
    let height = |c: &BodyContact| ((rm.transpose() * (c.point - st.pos)).y - hull.min.y) / (hull.max.y - hull.min.y).max(1e-3);
    let mut touch: Vec<Material> = found.iter().map(|c| material(c.surface, c.normal, height(c))).collect();
    if let Some((friction, elasticity, lower)) = head_on {
        for (c, m) in found.iter_mut().zip(touch.iter_mut()) {
            if c.normal.dot(back) > 0.906_307_8 {
                *m = (friction, elasticity);
                c.point.y -= lower;
            }
        }
    }
    let bounce = touch.iter().map(|m| m.1).sum::<f32>() / touch.len() as f32;
    let mut hardest: Option<(f32, Vec3)> = None;
    for c in found.iter() {
        let vn = (st.vel + st.ang_vel.cross(c.point - com)).dot(c.normal);
        if vn < 0.0 && hardest.map_or(true, |h| -vn > h.0) {
            hardest = Some((-vn, c.point));
        }
    }
    let literal = std::env::var_os("MCLA_DBG_OLDSOLVE").is_none();
    let (mut pv, mut pw) = (Vec3::ZERO, Vec3::ZERO);
    if literal {
        // The original's contact solver (0x825E8F88 with the kernel 0x825E51E8): every contact
        // is a row of three - along its normal and two ways across it - solved as one block
        // so that the point would stick, then brought back on to friction's cone; four sweeps
        // over the rows, each step taken 1.1 times over; and what the rows have given is
        // applied times 1 + the contacts' mean elasticity (0x825DCA18). A contact not yet
        // touching may close by what is left of its 6 mm in the tick.
        // (each at the BODY's point of it, contact +48 - the hull's own corner, as deep in
        // the wall as the contact is: 33 cm farther forward than the wall's face on an R8 that
        // met one at 26 m/s, and that much more arm for the blow to turn the car with)
        let rows: Vec<Row> = found.iter().zip(touch.iter()).map(|(c, m)| Row::new(c.normal, c.point - c.normal * c.depth - com, inv_m, inv_i, m.0)).collect();
        let bias: Vec<Vec3> = found.iter().map(|c| Vec3::new((0.007 - 0.001 - c.depth).max(0.0) / dt, 0.0, 0.0)).collect();
        // (each starts from what the same contact - the body's point of it within 2 cm -
        // ended with last tick, 0x825A37A8)
        let inv = rm.transpose();
        let local: Vec<Vec3> = rows.iter().map(|r| inv * (r.r + com - st.pos)).collect();
        let warm = std::env::var_os("MCLA_DBG_NOWARM").is_none();
        let before_it = |which: usize| -> Vec<Vec3> {
            local.iter().map(|p| kept.iter().filter(|_| warm).map(|k| ((k.0 - *p).length_squared(), if which == 0 { k.1 } else { k.2 })).filter(|k| k.0 < 0.0004).min_by(|a, b| a.0.total_cmp(&b.0)).map_or(Vec3::ZERO, |k| k.1)).collect()
        };
        let (mut given, mut pushed) = (before_it(0), before_it(1));
        // (the forces of this tick are in the velocities already here; the original, which
        // solves before it adds them, counts what they will add over 1 + elasticity)
        let less = 1.0 - 1.0 / (1.0 + bounce);
        let (dv, dw) = sweep(&rows, &bias, st.vel - pending.0 * less, st.ang_vel - pending.1 * less, inv_m, inv_i, 1.1, &mut given);
        st.vel += dv * (1.0 + bounce);
        st.ang_vel += dw * (1.0 + bounce);
        // out of what it stands in - the push solve (0x825DCEB8 with the same kernel, from
        // rest, taken once over): every contact asks to be let out by D = its depth past
        // 7 mm, less half of that (7 mm of it at most); the answer MOVES the body and TURNS it
        // about its place, without touching its speeds
        // (the deep case, 0x825DCEB8: a contact that asks for more than the body is thick
        // beyond it along the normal moves the body out directly by all but half that
        // thickness, and asks only for the half)
        let mut direct = Vec3::ZERO;
        let want: Vec<Vec3> = found.iter().map(|c| {
            let d = c.depth - 0.007;
            let mut ask = d - (0.5 * d).clamp(0.0, 0.007);
            let at = c.point - c.normal * c.depth;
            let thick = hull.verts.iter().map(|v| (st.pos + rm * *v - at).dot(c.normal)).fold(0.0f32, f32::max) + 0.04;
            if ask > 0.0 && ask > thick {
                direct += c.normal * (ask - thick * 0.5);
                ask = thick * 0.5;
            }
            Vec3::new(-ask / dt, 0.0, 0.0)
        }).collect();
        st.pos += direct;
        (pv, pw) = sweep(&rows, &want, Vec3::ZERO, Vec3::ZERO, inv_m, inv_i, 1.0, &mut pushed);
        *kept = local.iter().zip(given.iter().zip(pushed.iter())).map(|(p, (a, b))| (*p, *a, *b)).collect();
    } else {
        // (what each contact has given: along its normal, and across it)
        let mut given = vec![(0.0f32, Vec3::ZERO); found.len()];
        for round in 0..4 {
            for (i, c) in found.iter().enumerate() {
                let r = c.point - com;
                let v = st.vel + st.ang_vel.cross(r);
                let vn = v.dot(c.normal);
                if round == 0 && vn < 0.0 && hardest.map_or(true, |h| -vn > h.0) {
                    hardest = Some((-vn, c.point));
                }
                // along the normal: the closing speed goes (a shallow contact may close by what is
                // left of its 6 mm in the tick)
                let allowed = (0.006 - c.depth).max(0.0) / dt;
                let k = inv_m + (inv_i * r.cross(c.normal)).cross(r).dot(c.normal);
                let j = (-(vn + allowed) / k).max(-given[i].0);
                given[i].0 += j;
                let push = c.normal * j;
                st.vel += push * inv_m;
                st.ang_vel += inv_i * r.cross(push);
                // across it: friction's cone
                let v = st.vel + st.ang_vel.cross(r);
                let slide = v - c.normal * v.dot(c.normal);
                let speed = slide.length();
                if speed > 1e-4 {
                    let t = slide / speed;
                    let kt = inv_m + (inv_i * r.cross(t)).cross(r).dot(t);
                    let wanted = given[i].1 - t * (speed / kt);
                    let limit = touch[i].0 * given[i].0;
                    let held = if wanted.length() > limit { wanted * (limit / wanted.length()) } else { wanted };
                    let drag = held - given[i].1;
                    given[i].1 = held;
                    st.vel += drag * inv_m;
                    st.ang_vel += inv_i * r.cross(drag);
                }
            }
        }
        // (0x825DCA18: the impulses are applied times 1 + the contacts' mean elasticity)
        if bounce > 0.0 {
            for (i, c) in found.iter().enumerate() {
                let r = c.point - com;
                let extra = (c.normal * given[i].0 + given[i].1) * bounce;
                st.vel += extra * inv_m;
                st.ang_vel += inv_i * r.cross(extra);
            }
        }
        // out of what it stands in - the push solve (0x82591108: rows 0x825DCEB8, the velocity
        // pass's kernel again from rest, applied by 0x825DDCE8 / 0x8259E790): every contact asks
        // to be let out by D = its depth past 7 mm, less half of that (7 mm of it at most), and
        // the answer is a push at the contact - which MOVES the body and TURNS it about its place,
        // without touching its speeds
        let want: Vec<f32> = found.iter().map(|c| { let d = c.depth - 0.007; (d - (0.5 * d).clamp(0.0, 0.007)) / dt }).collect();
        let mut pushed = vec![(0.0f32, Vec3::ZERO); found.len()];
        for _ in 0..4 {
            for (i, c) in found.iter().enumerate() {
                let r = c.point - com;
                let k = inv_m + (inv_i * r.cross(c.normal)).cross(r).dot(c.normal);
                let vn = (pv + pw.cross(r)).dot(c.normal);
                let j = ((want[i] - vn) / k).max(-pushed[i].0);
                pushed[i].0 += j;
                let push = c.normal * j;
                pv += push * inv_m;
                pw += inv_i * r.cross(push);
                // (and it does not slide while it is pushed, as far as friction holds)
                let v = pv + pw.cross(r);
                let slide = v - c.normal * v.dot(c.normal);
                let speed = slide.length();
                if speed > 1e-6 {
                    let t = slide / speed;
                    let kt = inv_m + (inv_i * r.cross(t)).cross(r).dot(t);
                    let wanted = pushed[i].1 - t * (speed / kt);
                    let limit = touch[i].0 * pushed[i].0;
                    let held = if wanted.length() > limit { wanted * (limit / wanted.length().max(1e-9)) } else { wanted };
                    let drag = held - pushed[i].1;
                    pushed[i].1 = held;
                    pv += drag * inv_m;
                    pw += inv_i * r.cross(drag);
                }
            }
        }
    }
    let turn = pw * dt;
    if turn.length_squared() > 1e-14 && std::env::var_os("MCLA_DBG_NOPUSHTURN").is_none() {
        let q = glam::Quat::from_axis_angle(turn.normalize(), turn.length());
        // (about the body's place - the point its contacts turn it about)
        st.pos = com + q * (st.pos - com);
        st.rot = (q * st.rot).normalize();
    }
    st.pos += pv * dt;
    if st.vel != before.0 || st.ang_vel != before.1 {
        HIT.with(|t| t.set(true));
        WALLS.with(|t| t.set(found.iter().filter(|c| c.normal.y.abs() < 0.1).count() as u32));
    }
    if std::env::var_os("MCLA_DBG_HIT").is_some() {
        let inv = rm.transpose();
        eprintln!("HITSUM dv local {:?} dw local {:?} moved {:?}", inv * (st.vel - before.0), inv * (st.ang_vel - before.1), inv * (st.pos - before.2));
    }
    hardest
}

// ---------------------------------------------------------------------------------------
// The hull against the city as the original collides it, operation for operation: the
// polygons its bound is tried against and what each gives (0x825D0C18, 0x825CEE38, the sink
// 0x825C65E8), the contacts it keeps from tick to tick (0x825A3070), and what the car's own
// callback makes of them before the solver has them (0x82332D30). The solver: `solver.rs`.
// ---------------------------------------------------------------------------------------

use super::exact::{dot3, fma, recip};
use super::gjk::rsqrt_as_original as rs;
use super::solver::Contact;

/// One contact the hull keeps (the manifold's 176-byte record with what the port adds).
#[derive(Clone, Copy, Debug)]
pub struct Slot {
    pub c: Contact,
    /// ticks since it was made (+174): 0 until the tick's refresh has seen it
    pub age: u16,
    /// byte +165's top bit: the solver has it
    pub enabled: bool,
    pub surface: Surface,
    /// the polygon it is on (its number in its cell)
    pub polygon: u32,
    /// which of the hull's triangles it is on (the contact's element number, +168)
    pub element: u16,
    /// which of the car's parts it is on: 0 the hull, 1.. the wheels
    pub part: u8,
}

/// The hull's contacts with one cell of the city (the original keeps six at most).
#[derive(Clone, Debug, Default)]
pub struct Manifold {
    /// which of the car's parts: 0 the hull, 1.. the wheels (each part has its own record with a cell)
    pub part: u8,
    pub cell: usize,
    pub slots: Vec<Slot>,
}

pub(crate) const POLYGON_MARGIN: f32 = f32::from_bits(0x3ba3_d70a);
const PART_MARGIN: f32 = f32::from_bits(0x3d23_d70a);
const MILLIONTH: f32 = f32::from_bits(0x3586_37bd);

/// 0x825CED90: is `p` within the square root of `r2` of the edge from `a` to `b`?
fn beside_edge(a: Vec3, b: Vec3, p: Vec3, r2: f32) -> bool {
    let d = p - a;
    let ab = b - a;
    let mut q = Vec3::ZERO - d;
    let t = 0.0 - dot3(ab, q);
    if !(f32::from_bits(0x2b8c_bccc) >= t) {
        let rest = dot3(ab, ab) - t;
        if f32::from_bits(0x2b8c_bccc) >= rest {
            q = q + ab;
        } else {
            q = ab * (t * recip(t + rest)) + q;
        }
    }
    let off = (q + d) - d;
    r2 >= dot3(off, off)
}

/// The vector unit's arc cosine (0x825CEB68): sqrt(1 - |x|) times a polynomial of |x|, taken
/// from pi for a negative x.
pub fn vec_acos(x: f32) -> f32 {
    let f = f32::from_bits;
    let a = x.abs();
    let rest = 1.0 - a;
    let hi = ((f(0xbaa5_7a2c) * a + f(0x3bda_90c5)) * a + f(0xbc8b_fc66)) * a + f(0x3cfd_10f8);
    let lo = ((f(0xbd4d_8392) * a + f(0x3db6_3a9e)) * a + f(0xbe5b_bfca)) * a + f(0x3fc9_0fda);
    let a2 = a * a;
    let p = hi * (a2 * a2) + lo;
    let s = if rest == 0.0 { 0.0 } else { rest * rs(rest) };
    if x.is_sign_negative() { f(0x4049_0fdb) - s * p } else { s * p }
}

/// The vector unit's cosine (0x825CEAC8): the sine of what is left of x past a half turn's middle.
pub fn vec_cos(x: f32) -> f32 {
    let f = f32::from_bits;
    let fl = (x * f(0x3ea2_f983)).floor();
    let y = f(0xc049_0fdb) * (fl + 0.5) + x;
    let y2 = y * y;
    let p = (((y2 * f(0x3638_ef1d) + f(0xb950_0d01)) * y2 + f(0x3c08_8889)) * y2 + f(0xbe2a_aaab)) * y2 + 1.0;
    let s = y * p;
    if (fl as i32) & 1 == 0 { 0.0 - s } else { s }
}

/// `v` turned about `about` (any length) by twice `half`, as the cone routines do it: a
/// quaternion from the vector unit's sine / cosine, and the two products q v q* as sums of four.
fn quat_turned(v: Vec3, about: Vec3, half: f32) -> Vec3 {
    let (s, c) = super::solver::sin_cos(half);
    let u = about * rs(dot3(about, about));
    let q = [u.x * s, u.y * s, u.z * s, c];
    // (Q x V for V = (v, w): each a sum of four products, w and z first)
    let neg = |x: f32| 0.0 - x;
    let d = |v: [f32; 4], q: [f32; 4]| (v[3] * q[0] + v[2] * q[1]) + (neg(v[1]) * q[2] + v[0] * q[3]);
    let b = |v: [f32; 4], q: [f32; 4]| (v[3] * q[1] + neg(v[2]) * q[0]) + (v[1] * q[3] + v[0] * q[2]);
    let cc = |v: [f32; 4], q: [f32; 4]| (v[3] * q[2] + v[2] * q[3]) + (v[1] * q[0] + neg(v[0]) * q[1]);
    let a = |v: [f32; 4], q: [f32; 4]| (v[3] * q[3] + neg(v[2]) * q[2]) + (neg(v[1]) * q[1] + neg(v[0]) * q[0]);
    let v4 = [v.x, v.y, v.z, 0.0];
    let t = [d(v4, q), b(v4, q), cc(v4, q), a(v4, q)];
    let conj = [neg(q[0]), neg(q[1]), neg(q[2]), q[3]];
    Vec3::new(d(conj, t), b(conj, t), cc(conj, t))
}

/// 0x825D0600: the cone (`axis`, cosine of its half angle) widened to hold the direction `other`.
pub fn cone_widen(cos: &mut f32, axis: Vec3, other: Vec3) -> Vec3 {
    let d = dot3(axis, other);
    if !(*cos > d) {
        return axis;
    }
    if *cos >= f32::from_bits(0x3f7f_f972) {
        let h = (axis + other) * 0.5;
        let x = 0.5 * d + 0.5;
        let l2 = dot3(h, h);
        *cos = if 0.0 >= x { 0.0 } else { x * rs(x) };
        return if l2 > MILLIONTH { h * rs(l2) } else { Vec3::Z };
    }
    let a1 = vec_acos(d.max(-1.0).min(1.0));
    let a2 = vec_acos(cos.max(-1.0).min(1.0));
    let about = super::exact::cross(other, axis);
    let turned = quat_turned(axis, about, ((a2 - a1) * 0.5) * 0.5);
    *cos = vec_cos((a2 + a1) * 0.5);
    turned
}

/// 0x825D0950: `n` if it lies inside the cone, else the cone's rim nearest it.
pub fn cone_clamp(cos: f32, n: Vec3, axis: Vec3) -> Vec3 {
    if !(cos - f32::from_bits(0x38d1_b717) > dot3(n, axis)) {
        return n;
    }
    if cos >= f32::from_bits(0x3f7f_f972) {
        return axis;
    }
    let about = super::exact::cross(axis, n);
    quat_turned(axis, about, vec_acos(cos.max(-1.0).min(1.0)) * 0.5)
}

/// The contact's normal as 0x825D0C18 leaves it, operation for operation: `tri` the polygon's
/// corners and `on_b` the contact's point on it, both in the world.
pub fn cone_exact(tri: &[Vec3; 3], face: Vec3, across: &[Option<Vec3>; 3], on_b: Vec3, n: Vec3) -> Vec3 {
    let r2 = (f32::from_bits(0x3f85_2bd3) * POLYGON_MARGIN) * POLYGON_MARGIN;
    let (mut axis, mut cos) = (face, 1.0f32);
    for e in 0..3 {
        let Some(other) = across[e] else { continue };
        if beside_edge(tri[e], tri[(e + 1) % 3], on_b, r2) {
            axis = cone_widen(&mut cos, axis, other);
        }
    }
    cone_clamp(cos, n, axis)
}

/// 0x825CEE38's test of a contact against the polygon's edges, operation for operation:
/// `about` the corners about the first, `edge` per edge the direction between the two faces
/// and the neighbour's normal where they meet in a ridge (else the face's own, twice),
/// `on_b` the polygon's point about the first corner.
pub fn edge_test(about: &[Vec3; 3], has: [bool; 3], edge: &[(Vec3, Vec3); 3], on_b: Vec3, normal: Vec3) -> Option<usize> {
    let r2 = (f32::from_bits(0x3f85_2bd3) * POLYGON_MARGIN) * POLYGON_MARGIN;
    for e in 0..3 {
        if !has[e] || !beside_edge(about[e], about[(e + 1) % 3], on_b, r2) {
            continue;
        }
        let ed = about[(e + 1) % 3] - about[e];
        let ed = ed * rs(dot3(ed, ed));
        let k = dot3(normal, ed);
        let lean = if k == 0.0 {
            normal
        } else {
            let q = -(ed * k - normal);
            q * rs(dot3(q, q))
        };
        let least = dot3(edge[e].0, edge[e].1).min(1.0 - MILLIONTH) - f32::from_bits(0x3a83_126f);
        if least > dot3(lean, edge[e].0) {
            return Some(e);
        }
    }
    None
}

/// MCLA_DBG_OLDGRID: every cell whose triangles reach the box, as before stage Y3
fn grid_old() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var_os("MCLA_DBG_OLDGRID").is_some())
}

fn old_cone() -> bool {
    static D: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *D.get_or_init(|| std::env::var_os("MCLA_DBG_OLDCONE").is_some())
}

/// The record of one part with one cell, made where it is missing: the parts in order, the
/// cells in order within a part.
fn manifold_of(manifolds: &mut Vec<Manifold>, part: u8, cell: usize) -> usize {
    match manifolds.iter().position(|m| m.part == part && m.cell == cell) {
        Some(k) => k,
        None => {
            let k = manifolds.iter().position(|m| (m.part, m.cell) > (part, cell)).unwrap_or(manifolds.len());
            manifolds.insert(k, Manifold { part, cell, slots: Vec::new() });
            k
        }
    }
}

/// Everything one wheel's shape touches, laid into `manifolds` as the hull's contacts are
/// (0x825CCE00 for a wheel's part): the same narrow phase, edge test and cone as `wheel_touch`,
/// every contact kept. `now`: the body's matrix (rows and the model's origin).
pub fn wheel_collide(world: &WorldCollider, core: &super::gjk::WheelCore, part: u8, now: &[Vec3; 4], ball: Option<(Vec3, f32)>, manifolds: &mut Vec<Manifold>, moved: Option<([Vec3; 4], [Vec3; 4], Vec3, [Vec3; 4])>) {
    let reach = Vec3::splat(core.round() + CORE + 0.03);
    // (RA6, 0x825CE970 as for the hull's part: the box a polygon has to cut - the wheel part's own box as it
    // stands, grown to take in the same box at the part's "last" matrix, and the mesh's 5 mm)
    let cut = moved.as_ref().map(|(now, last, half, _)| {
        let (c, r) = (now[3], [now[0], now[1], now[2]]);
        let d = last[3] - c;
        let q = Vec3::new(d.dot(r[0]), d.dot(r[1]), d.dot(r[2]));
        let e_last = Vec3::from_array(std::array::from_fn(|i| (0..3).map(|j| r[i].dot(last[j]).abs() * half[j]).sum()));
        let (lo, hi) = ((-*half).min(q - e_last), half.max(q + e_last));
        let m = (lo + hi) * 0.5;
        WheelBox { centre: c + r[0] * m.x + r[1] * m.y + r[2] * m.z, axes: r, half: (hi - lo) * 0.5 + Vec3::splat(super::collision::WORLD_MARGIN) }
    });
    let ball = ball.filter(|_| !grid_old());
    world.triangles_by_cell(core.centre - reach, core.centre + reach, ball, |cell, mesh, i| {
        if mesh.faces.len() != mesh.tris.len() {
            return;
        }
        let (tri, face) = (mesh.tris[i], mesh.faces[i]);
        if face == Vec3::ZERO {
            return;
        }
        if cut.as_ref().is_some_and(|b| !b.touches(&tri, face)) {
            return;
        }
        let other: [Option<Vec3>; 3] = mesh.beside[i].map(|n| n.map(|j| mesh.faces[j as usize]));
        let edge: [(Vec3, Vec3); 3] = std::array::from_fn(|e| match other[e] {
            Some(nn) => {
                let (b, ridge) = super::gjk::edge_bisector(face, nn, tri[e], tri[(e + 1) % 3]);
                (b, if ridge { nn } else { face })
            }
            None => (face, face),
        });
        let Some(r) = super::gjk::contact(core, &tri, face, [edge[0].0, edge[1].0, edge[2].0], super::gjk::WHEEL_MARGIN, super::collision::WORLD_MARGIN) else { return };
        if r.distance >= 0.02 {
            return;
        }
        let about = [Vec3::ZERO, tri[1] - tri[0], tri[2] - tri[0]];
        if edge_test(&about, other.map(|o| o.is_some()), &edge, r.on_b - tri[0], r.normal).is_some() {
            return;
        }
        // (RA6: the hull's "moving out" test, 0x825CEE38 (5), for the wheel's point as the CAR carries it: where
        // that point of the body has gone since the car's last matrix, along the polygon's normal it must not
        // have come out. I + V sa.0 216..231: with the part's own "last" matrix - the top of its travel - the
        // recorded contacts of 223 / 225 would be refused.)
        if let Some((pn, pb, _, _)) = moved.as_ref() {
            let from = r.on_a - pn[3];
            let u = Vec3::new(dot3(pn[0], from), dot3(pn[1], from), dot3(pn[2], from));
            let delta = [pn[0] - pb[0], pn[1] - pb[1], pn[2] - pb[2], pn[3] - pb[3]];
            let mv = Vec3::new(
                (delta[3].x + delta[2].x * u.z) + (delta[1].x * u.y + delta[0].x * u.x),
                (delta[3].y + delta[2].y * u.z) + (delta[1].y * u.y + delta[0].y * u.x),
                (delta[3].z + delta[2].z * u.z) + (delta[1].z * u.y + delta[0].z * u.x),
            );
            if std::env::var_os("MCLA_DBG_WTOUCH").is_some() {
                eprintln!("WMOVE part {part} polygon {i} out {:?} dist {:?} pn {:?} pl {:?}", dot3(mv, face), r.distance, pn, pb);
            }
            if dot3(mv, face) > f32::from_bits(0x38d1_b717) {
                return;
            }
        }
        let normal = cone_exact(&tri, face, &other, r.on_b, r.normal);
        if std::env::var_os("MCLA_DBG_WTOUCH").is_some() {
            eprintln!("WC part {part} cell {cell} polygon {i} depth {:?} gjk n {:?} cone {:?} face {:?} on_b {:?} tri {:?} beside {:?}", 0.0 - r.distance, r.normal, normal, face, r.on_b, tri, mesh.beside[i]);
        }
        let from = r.on_a - now[3];
        let local = Vec3::new(dot3(now[0], from), dot3(now[1], from), dot3(now[2], from));
        let fresh = Contact { depth: 0.0 - r.distance, local, on_body: r.on_a, on_world: r.on_b, normal, ..Default::default() };
        let at = manifold_of(manifolds, part, cell);
        let m = &mut manifolds[at];
        let mut near = (f32::from_bits(0x39d1_b717), None);
        for (k, s) in m.slots.iter().enumerate() {
            let d = s.c.local - local;
            let d2 = fma(d.x, d.x, fma(d.z, d.z, d.y * d.y));
            if d2 < near.0 && s.age != 0 {
                near = (d2, Some(k));
            }
        }
        let surface = Surface::from_u8(mesh.mats.get(i).copied().unwrap_or(0));
        let slot = Slot { c: fresh, age: 0, enabled: true, surface, polygon: i as u32, element: 0, part };
        match near.1 {
            Some(k) => {
                let old = m.slots[k];
                m.slots[k] = Slot { c: Contact { kept: old.c.kept, pushed: old.c.pushed, ..fresh }, ..slot };
            }
            None if m.slots.len() < 6 => m.slots.push(slot),
            None => {}
        }
    });
    manifolds.retain(|m| !m.slots.is_empty());
}

/// What one convex part of a loose body touches of the city (stage P; 0x825CCE00 -> 0x825D0C18 /
/// 0x825CEE38 as for the hull's part): `core` the part's core at its matrix `now` (rows and
/// place), `margin` its own, `bbox` its bound's box in its own frame, `last` its matrix as the
/// last collide is to have had it. Each contact is handed to `hit`.
pub struct PartHit {
    pub cell: usize,
    pub polygon: u32,
    pub surface: Surface,
    /// the part's point in its own frame and in the world, the city's point, the way the part is pushed
    pub local: Vec3,
    pub on_a: Vec3,
    pub on_b: Vec3,
    pub normal: Vec3,
    pub depth: f32,
}

/// `swept`: the detector to use in place of the discrete one (0x825CEE38's swept branch, taken when the pair's
/// flag byte is set - a loose prop): handed the polygon about its first corner, its normal, its edges' bisectors
/// and that corner; it answers about the corner, as the discrete one does.
pub fn part_touches(core: &impl super::gjk::Support, margin: f32, bbox: (Vec3, Vec3), now: &[Vec3; 4], last: &[Vec3; 4], world: &WorldCollider, swept: Option<&dyn Fn(&[Vec3; 3], Vec3, [Vec3; 3], Vec3) -> Option<super::gjk::Nearest>>, mut hit: impl FnMut(PartHit)) {
    let (mid, half) = ((bbox.0 + bbox.1) * 0.5, (bbox.1 - bbox.0) * 0.5);
    let centre = |m: &[Vec3; 4]| m[3] + m[0] * mid.x + m[1] * mid.y + m[2] * mid.z;
    let reach = {
        let (c, r) = (centre(now), [now[0], now[1], now[2]]);
        let d = centre(last) - c;
        let q = Vec3::new(d.dot(r[0]), d.dot(r[1]), d.dot(r[2]));
        let e_last = Vec3::from_array(std::array::from_fn(|i| (0..3).map(|j| r[i].dot(last[j]).abs() * half[j]).sum()));
        let (lo, hi) = ((-half).min(q - e_last), half.max(q + e_last));
        let m = (lo + hi) * 0.5;
        WheelBox { centre: c + r[0] * m.x + r[1] * m.y + r[2] * m.z, axes: r, half: (hi - lo) * 0.5 + Vec3::splat(super::collision::WORLD_MARGIN) }
    };
    let (mut mn, mut mx) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for i in 0..8 {
        let s = Vec3::new(if i & 1 == 0 { -1.0 } else { 1.0 }, if i & 2 == 0 { -1.0 } else { 1.0 }, if i & 4 == 0 { -1.0 } else { 1.0 }) * reach.half;
        let w = reach.centre + reach.axes[0] * s.x + reach.axes[1] * s.y + reach.axes[2] * s.z;
        mn = mn.min(w);
        mx = mx.max(w);
    }
    let delta = [now[0] - last[0], now[1] - last[1], now[2] - last[2], now[3] - last[3]];
    world.triangles_by_cell(mn, mx, None, |cell, mesh, i| {
        if mesh.faces.len() != mesh.tris.len() {
            return;
        }
        let (tri, face) = (mesh.tris[i], mesh.faces[i]);
        if face == Vec3::ZERO || !reach.touches(&tri, face) {
            return;
        }
        let other: [Option<Vec3>; 3] = mesh.beside[i].map(|n| n.map(|j| mesh.faces[j as usize]));
        let edge: [(Vec3, Vec3); 3] = std::array::from_fn(|e| match other[e] {
            Some(nn) => {
                let (b, ridge) = super::gjk::edge_bisector(face, nn, tri[e], tri[(e + 1) % 3]);
                (b, if ridge { nn } else { face })
            }
            None => (face, face),
        });
        let origin = tri[0];
        let bisectors = [edge[0].0, edge[1].0, edge[2].0];
        let found = match swept {
            Some(f) => f(&[Vec3::ZERO, tri[1] - origin, tri[2] - origin], face, bisectors, origin),
            None => super::gjk::contact_about(core, &tri, face, bisectors, margin, POLYGON_MARGIN),
        };
        let Some(r) = found else { return };
        let (on_a, on_b) = (r.on_a + origin, r.on_b + origin);
        let from = on_a - now[3];
        let local = Vec3::new(dot3(now[0], from), dot3(now[1], from), dot3(now[2], from));
        let moved = Vec3::new(
            (delta[3].x + delta[2].x * local.z) + (delta[1].x * local.y + delta[0].x * local.x),
            (delta[3].y + delta[2].y * local.z) + (delta[1].y * local.y + delta[0].y * local.x),
            (delta[3].z + delta[2].z * local.z) + (delta[1].z * local.y + delta[0].z * local.x),
        );
        if dot3(moved, face) > MILLIONTH {
            return;
        }
        let about = [Vec3::ZERO, tri[1] - origin, tri[2] - origin];
        if edge_test(&about, other.map(|o| o.is_some()), &edge, r.on_b, r.normal).is_some() {
            return;
        }
        if !(f32::from_bits(0x3ca3_d70a) > r.distance) {
            return;
        }
        let normal = cone_exact(&tri, face, &other, on_b, r.normal);
        hit(PartHit { cell, polygon: i as u32, surface: Surface::from_u8(mesh.mats.get(i).copied().unwrap_or(0)), local, on_a, on_b, normal, depth: 0.0 - r.distance });
    });
}

/// Everything the hull touches this tick is laid into `manifolds` (0x825CCE00 for the hull's
/// part): `now` and `last` the body's matrix - rows and the model's origin - as it stands and
/// as the last tick left it; `corners` the hull's corners, `bbox` its box in its own frame.
/// The ball the city's grid picks its cells by (0x825CD184..0x825CD694): the car's whole box - `lo`..`hi`
/// in its own frame, hull and wheels - as it stands, grown to take in the same box at the body's last
/// matrix; its middle in the world and the length of its half sizes. (A decision only: written plainly,
/// not operation for operation.)
pub fn swept_ball(now: &[Vec3; 4], last: &[Vec3; 4], lo: Vec3, hi: Vec3) -> (Vec3, f32) {
    let (mid, half) = ((lo + hi) * 0.5, (hi - lo) * 0.5);
    let centre = |m: &[Vec3; 4]| m[3] + m[0] * mid.x + m[1] * mid.y + m[2] * mid.z;
    let (c, r) = (centre(now), [now[0], now[1], now[2]]);
    let d = centre(last) - c;
    let q = Vec3::new(d.dot(r[0]), d.dot(r[1]), d.dot(r[2]));
    let e_last = Vec3::from_array(std::array::from_fn(|i| (0..3).map(|j| r[i].dot(last[j]).abs() * half[j]).sum()));
    let (a, b) = ((-half).min(q - e_last), half.max(q + e_last));
    let m = (a + b) * 0.5;
    (c + r[0] * m.x + r[1] * m.y + r[2] * m.z, ((b - a) * 0.5).length())
}

pub fn collide(hull: &HullShape, bbox: (Vec3, Vec3), now: &[Vec3; 4], last: &[Vec3; 4], world: &WorldCollider, ball: Option<(Vec3, f32)>, manifolds: &mut Vec<Manifold>) {
    let corners = &hull.verts[..];
    let dbg = { static D: std::sync::OnceLock<bool> = std::sync::OnceLock::new(); *D.get_or_init(|| std::env::var_os("MCLA_DBG_HIT").is_some()) };
    let core = super::gjk::HullCore { corners: hull.scanned(), rows: [now[0], now[1], now[2]], at: now[3] };
    // (the box a polygon has to cut, 0x825CE970: the hull's box as it stands grown to take in
    // the same box where it stood, and the mesh's 5 mm)
    let (mid, half) = ((bbox.0 + bbox.1) * 0.5, (bbox.1 - bbox.0) * 0.5);
    let centre = |m: &[Vec3; 4]| m[3] + m[0] * mid.x + m[1] * mid.y + m[2] * mid.z;
    let reach = {
        let (c, r) = (centre(now), [now[0], now[1], now[2]]);
        let d = centre(last) - c;
        let q = Vec3::new(d.dot(r[0]), d.dot(r[1]), d.dot(r[2]));
        let e_last = Vec3::from_array(std::array::from_fn(|i| (0..3).map(|j| r[i].dot(last[j]).abs() * half[j]).sum()));
        let (lo, hi) = ((-half).min(q - e_last), half.max(q + e_last));
        let m = (lo + hi) * 0.5;
        WheelBox { centre: c + r[0] * m.x + r[1] * m.y + r[2] * m.z, axes: r, half: (hi - lo) * 0.5 + Vec3::splat(super::collision::WORLD_MARGIN) }
    };
    let (mut mn, mut mx) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for i in 0..8 {
        let s = Vec3::new(if i & 1 == 0 { -1.0 } else { 1.0 }, if i & 2 == 0 { -1.0 } else { 1.0 }, if i & 4 == 0 { -1.0 } else { 1.0 }) * reach.half;
        let w = reach.centre + reach.axes[0] * s.x + reach.axes[1] * s.y + reach.axes[2] * s.z;
        mn = mn.min(w);
        mx = mx.max(w);
    }
    let r2 = (f32::from_bits(0x3f85_2bd3) * POLYGON_MARGIN) * POLYGON_MARGIN;
    let delta = [now[0] - last[0], now[1] - last[1], now[2] - last[2], now[3] - last[3]];
    let ball = ball.filter(|_| !grid_old());
    world.triangles_by_cell(mn, mx, ball, |cell, mesh, i| {
        if mesh.faces.len() != mesh.tris.len() {
            return;
        }
        let (tri, face) = (mesh.tris[i], mesh.faces[i]);
        if face == Vec3::ZERO || !reach.touches(&tri, face) {
            return;
        }
        // (0x82576FF8: across each edge the neighbour's normal, and between the two where
        // they meet in a ridge)
        let other: [Option<Vec3>; 3] = mesh.beside[i].map(|n| n.map(|j| mesh.faces[j as usize]));
        let edge: [(Vec3, Vec3); 3] = std::array::from_fn(|e| match other[e] {
            Some(nn) => {
                let (b, ridge) = super::gjk::edge_bisector(face, nn, tri[e], tri[(e + 1) % 3]);
                (b, if ridge { nn } else { face })
            }
            None => (face, face),
        });
        let found = super::gjk::contact_about(&core, &tri, face, [edge[0].0, edge[1].0, edge[2].0], PART_MARGIN, POLYGON_MARGIN);
        // (MCLA_DBG_HULLGJK=<file>: every call as a case of tools/hull_reference.py, the answer here after the '#')
        {
            use std::io::Write;
            static OUT: std::sync::OnceLock<Option<std::sync::Mutex<std::fs::File>>> = std::sync::OnceLock::new();
            if let Some(out) = OUT.get_or_init(|| std::env::var_os("MCLA_DBG_HULLGJK").and_then(|p| std::fs::OpenOptions::new().create(true).append(true).open(p).ok()).map(std::sync::Mutex::new)) {
                let mut line = format!("{:x}", core.corners.len());
                let all = core.corners.iter().chain(core.rows.iter()).chain([&core.at]).chain(tri.iter()).chain([&face, &edge[0].0, &edge[1].0, &edge[2].0]);
                for v in all {
                    line.push_str(&format!(" {:08x} {:08x} {:08x}", v.x.to_bits(), v.y.to_bits(), v.z.to_bits()));
                }
                match found {
                    Some(r) => line.push_str(&format!(" # 1,{},{:08x},{:08x},{:08x},{:08x},{:08x},{:08x},{:08x},{:08x},{:08x},{:08x}", r.deep as u8, r.normal.x.to_bits(), r.normal.y.to_bits(), r.normal.z.to_bits(), r.on_a.x.to_bits(), r.on_a.y.to_bits(), r.on_a.z.to_bits(), r.on_b.x.to_bits(), r.on_b.y.to_bits(), r.on_b.z.to_bits(), r.distance.to_bits())),
                    None => line.push_str(" # 0"),
                }
                let _ = writeln!(out.lock().unwrap(), "{line}");
            }
        }
        let Some(r) = found else { return };
        let origin = tri[0];
        let (on_a, on_b) = (r.on_a + origin, r.on_b + origin);
        // (the body's point of it, in the body's frame)
        let from = on_a - now[3];
        let local = Vec3::new(dot3(now[0], from), dot3(now[1], from), dot3(now[2], from));
        if dbg {
            eprintln!("HULLPOLY cell {cell} polygon {i} face {:?} depth {:?} n {:?} local {:?} on_b {:?} deep {}", face, -r.distance, r.normal, local, on_b, r.deep);
        }
        // (where that point of the body has gone since the last tick: along the polygon's
        // normal it must not have come out)
        let moved = Vec3::new(
            (delta[3].x + delta[2].x * local.z) + (delta[1].x * local.y + delta[0].x * local.x),
            (delta[3].y + delta[2].y * local.z) + (delta[1].y * local.y + delta[0].y * local.x),
            (delta[3].z + delta[2].z * local.z) + (delta[1].z * local.y + delta[0].z * local.x),
        );
        if dot3(moved, face) > MILLIONTH {
            if dbg { eprintln!("HULLPOLY   moving out: none"); }
            return;
        }
        // (the edges with a neighbour that the polygon's point lies on: the contact's
        // direction, taken across the edge, must lie within what the two faces allow)
        let about = [Vec3::ZERO, tri[1] - origin, tri[2] - origin];
        if let Some(e) = edge_test(&about, other.map(|o| o.is_some()), &edge, r.on_b, r.normal) {
            if dbg { eprintln!("HULLPOLY   edge {e}: none"); }
            return;
        }
        // (the sink: nothing farther than 2 cm)
        if !(f32::from_bits(0x3ca3_d70a) > r.distance) {
            return;
        }
        let normal = if old_cone() { cone_normal(&tri, face, &other, on_b, r.normal) } else { cone_exact(&tri, face, &other, on_b, r.normal) };
        let fresh = Contact { depth: 0.0 - r.distance, local, on_body: on_a, on_world: on_b, normal, ..Default::default() };
        let at = manifold_of(manifolds, 0, cell);
        let m = &mut manifolds[at];
        // (0x825A37A8: the contact it replaces - the nearest, by the body's point, within 2 cm
        // among those a refresh has seen)
        let mut near = (f32::from_bits(0x39d1_b717), None);
        for (k, s) in m.slots.iter().enumerate() {
            let d = s.c.local - local;
            let d2 = fma(d.x, d.x, fma(d.z, d.z, d.y * d.y));
            if d2 < near.0 && s.age != 0 {
                near = (d2, Some(k));
            }
        }
        let surface = Surface::from_u8(mesh.mats.get(i).copied().unwrap_or(0));
        // (which of the hull's triangles: by the corners the loop ended on, 0x825E1E10)
        let back = Vec3::ZERO - r.normal;
        let element = hull.element(&r.corners.0[..r.corners.1], Vec3::new(dot3(now[0], back), dot3(now[1], back), dot3(now[2], back)));
        let slot = Slot { c: fresh, age: 0, enabled: true, surface, polygon: i as u32, element, part: 0 };
        match near.1 {
            Some(k) => {
                // (the one it replaces hands on its multipliers, and - if it was the deeper - its element)
                let old = m.slots[k];
                m.slots[k] = Slot { c: Contact { kept: old.c.kept, pushed: old.c.pushed, ..fresh }, element: if old.c.depth >= fresh.depth { old.element } else { element }, ..slot };
            }
            None if m.slots.len() < 6 => m.slots.push(slot),
            // (the seventh, 0x825A32C0: it takes the place of the kept one that spans least - or is not taken.
            // MCLA_DBG_RA3: never taken, as before.)
            None => {
                let k = seventh_contact(std::array::from_fn(|i| m.slots[i].c.local), local);
                if k < 6 && !super::vehsim::dbg_flag("MCLA_DBG_RA3") {
                    m.slots[k] = slot;
                }
            }
        }
    });
    // (a cell's record that ends the pass with nothing in it is given up)
    manifolds.retain(|m| !m.slots.is_empty());
}

/// 0x825A32C0: a manifold holds six contacts and a seventh comes (`kept`, `new`: their points in side A's
/// frame). Gives the number of the kept contact the new one takes the place of, or 6: it is not taken.
/// The seven points are laid out around the one that lies farthest one way in their plane; the new one is
/// dropped if it lies inside the others' outline, else the kept one whose corner spans the least goes.
pub fn seventh_contact(kept: [Vec3; 6], new: Vec3) -> usize {
    use super::exact::cross;
    let n = 6usize;
    let mut p: [Vec3; 7] = [kept[0], kept[1], kept[2], kept[3], kept[4], kept[5], new];
    let mut id: [usize; 7] = [0, 1, 2, 3, 4, 5, 6];
    let e = p[1] - p[0];
    let len2 = |v: Vec3| fma(v.z, v.z, fma(v.x, v.x, v.y * v.y));
    let mut nn = cross(e, p[2] - p[0]);
    let mut l = len2(nn);
    let mut k = 3;
    while l < f32::from_bits(0x3586_37bd) && k < n {
        nn = cross(e, p[k] - p[0]);
        l = len2(nn);
        k += 1;
    }
    let t = cross(nn, e);
    // (the start: as the original has it, the POINT's coordinates become the bound, not its measure)
    let mut best = Vec3::splat(f32::from_bits(0x7149_f2ca));
    let mut b = 0;
    for i in 0..=n {
        let m = dot3(t, p[i]);
        if m >= best.x && m >= best.y && m >= best.z {
            continue;
        }
        best = p[i];
        b = i;
    }
    p.swap(b, 0);
    id.swap(b, 0);
    let p0 = p[0];
    // (the others in order around it)
    let mut passes = 0;
    loop {
        let mut swapped = false;
        for j in 1..n {
            let s = dot3(nn, cross(p[j] - p0, p[j + 1] - p0));
            if !(s >= 0.0) {
                p.swap(j, j + 1);
                id.swap(j, j + 1);
                swapped = true;
            }
        }
        passes += 1;
        if !(swapped && passes < n) {
            break;
        }
    }
    // (the corner that spans least)
    let m = n + 1;
    let mut least = f32::from_bits(0x7149_f2ca);
    let mut r = 0;
    for i in 1..=m {
        let (prev, cur, next) = (p[i - 1], p[i % m], p[(i + 1) % m]);
        let s = dot3(nn, cross(cur - prev, next - prev));
        if id[i % m] == n {
            if s >= 0.0 {
                continue;
            }
            return n;
        }
        if s >= least {
            continue;
        }
        least = s;
        r = i % m;
    }
    id[r]
}

impl HullShape {
    /// A triangle's neighbour across the edge between two of its corners (the file's record
    /// names it; here it is looked for).
    fn across(&self, t: usize, a: u16, b: u16) -> Option<usize> {
        self.tris.iter().enumerate().position(|(k, o)| k != t && o.contains(&a) && o.contains(&b))
    }

    fn outward(&self, t: usize) -> Vec3 {
        let [a, b, c] = self.tris[t].map(|i| self.verts[i as usize]);
        super::gjk::cross_as_original(c - b, a - b).normalize_or_zero()
    }

    /// 0x825E10A0: the triangle a contact is on, from the corners (their numbers, each once)
    /// the detector's simplex ended with and the contact's direction in the hull's frame: of
    /// one corner, the triangle about it that faces that way most; of two, the one of the two
    /// along that edge that does; of three, the triangle they are.
    pub fn element(&self, corners: &[u32], dir: Vec3) -> u16 {
        if self.mats.iter().collect::<std::collections::BTreeSet<_>>().len() <= 1 {
            return 0;
        }
        let mut ids: Vec<u16> = Vec::new();
        for c in corners {
            if !ids.contains(&(*c as u16)) {
                ids.push(*c as u16);
            }
        }
        let has = |t: &[u16; 3], v: u16| t.contains(&v);
        match ids.len() {
            1 => {
                let v = ids[0];
                let Some(first) = self.tris.iter().position(|t| has(t, v)) else { return 0 };
                let mut best = (dot3(self.outward(first), dir), first);
                // (round the corner, 0x8259F530: across the edge that ends in it, or - where
                // that leads back - the edge that begins in it)
                let (mut prev, mut at) = (usize::MAX, first);
                for _ in 0..self.tris.len() {
                    let t = self.tris[at];
                    let Some(i) = (0..3).find(|i| t[(i + 1) % 3] == v) else { break };
                    let mut next = self.across(at, t[i], t[(i + 1) % 3]);
                    if next.map_or(usize::MAX, |n| n) == prev {
                        next = self.across(at, t[(i + 1) % 3], t[(i + 2) % 3]);
                    }
                    let Some(next) = next else { break };
                    if next == first {
                        break;
                    }
                    let d = dot3(self.outward(next), dir);
                    if d > best.0 {
                        best = (d, next);
                    }
                    prev = at;
                    at = next;
                }
                best.1 as u16
            }
            2 => {
                let Some(first) = self.tris.iter().position(|t| has(t, ids[0]) && has(t, ids[1])) else { return 0 };
                match self.across(first, ids[0], ids[1]) {
                    Some(other) if !(dot3(self.outward(first), dir) > dot3(self.outward(other), dir)) => other as u16,
                    _ => first as u16,
                }
            }
            3 => self.tris.iter().position(|t| ids.iter().all(|v| has(t, *v))).unwrap_or(0) as u16,
            // (four corners, the jump table at 0x825E10F8: as three - the first triangle that holds three of them, 0x8259F4D8)
            4 => self.tris.iter().position(|t| ids.iter().filter(|v| has(t, **v)).count() >= 3).unwrap_or(0) as u16,
            _ => 0,
        }
    }

    /// What a triangle is made of.
    pub fn surface_of(&self, element: u16) -> Surface {
        match self.mats.get(element as usize).map(|m| m & 0xff) {
            Some(3) => Surface::CarRoof,
            Some(5) => Surface::CarUndercarriage,
            _ => Surface::CarSide,
        }
    }
}

/// The contacts kept from earlier ticks brought up to the body's matrix as it stands
/// (0x825A3070): their points, their depth; what has come 4 cm apart is dropped, and of those
/// a tick old or more the one that has slid farthest, if more than 2 cm.
pub fn refresh(now: &[Vec3; 4], manifolds: &mut Vec<Manifold>) {
    for m in manifolds.iter_mut() {
        for s in m.slots.iter_mut().rev() {
            if s.age != 0 {
                let l = s.c.local;
                s.c.on_body = Vec3::new(
                    (now[3].x + now[2].x * l.z) + (now[1].x * l.y + now[0].x * l.x),
                    (now[3].y + now[2].y * l.z) + (now[1].y * l.y + now[0].y * l.x),
                    (now[3].z + now[2].z * l.z) + (now[1].z * l.y + now[0].z * l.x),
                );
                s.c.depth = dot3(s.c.on_body - s.c.on_world, Vec3::ZERO - s.c.normal);
            }
            s.c.target = Vec3::ZERO;
            s.age = s.age.saturating_add(1);
        }
        let mut worst = (0.0f32, None);
        let mut k = 0;
        while k < m.slots.len() {
            let s = m.slots[k];
            if !(s.c.depth > f32::from_bits(0xbd23_d70a)) {
                m.slots.swap_remove(k);
                continue;
            }
            // (older than the count at 0x828127F0, which the game's own simulator sets to 1: 0x82740FB8)
            if s.age > 1 {
                let back = Vec3::ZERO - s.c.normal;
                let slid = s.c.on_world - (-(back * s.c.depth - s.c.on_body));
                let d2 = dot3(slid, slid);
                if !(worst.0 >= d2) && !(f32::from_bits(0x39d1_b717) >= d2) {
                    worst = (d2, Some(k));
                }
            }
            k += 1;
        }
        if let Some(k) = worst.1 {
            m.slots.swap_remove(k);
        }
    }
    manifolds.retain(|m| !m.slots.is_empty());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::collision::SectorMesh;

    fn data() -> std::path::PathBuf {
        std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR"))
    }

    /// The Saleen S281's hull as read from the running game (0xBF50E120) with its fitted
    /// wheels (radii 0.3312 / 0.3417): the floor, the bumper bar and one untouched point.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR containing Complete Edition vehicle resources"]
    fn hull_is_reshaped_as_the_running_game_reshapes_it() {
        let root = data();
        let assets = crate::assets::Assets::new(&root);
        let car = crate::vehicle_model::load_car(&assets, &root.join("resources/vehicle/vp_sln_s281_06")).unwrap();
        let file = car.hull.as_ref().expect("the fragment's hull");
        assert_eq!((file.verts.len(), file.tris.len()), (18, 32));
        let st = car.simtune.as_ref().unwrap();
        let mut tune = mcla_formats::rage::simtune::SimTune::lerp(&st.base, &st.mods, 1.0);
        tune.wheel_front.radius = 0.3312;
        tune.wheel_back.radius = 0.3417;
        let hubs = car.hubs.unwrap();
        let hull = shaped(file, &tune, (hubs[0].y + hubs[2].y) * 0.5, false);
        let near = |v: Vec3, want: [f32; 3]| (v - Vec3::from(want)).length() < 2e-3;
        assert!(near(hull.verts[0], [-0.770, 0.688, -2.245]), "{:?}", hull.verts[0]);
        assert!(near(hull.verts[10], [0.770, 0.688, -2.245]), "{:?}", hull.verts[10]);
        assert!(near(hull.verts[1], [0.721, 0.260, -1.948]), "{:?}", hull.verts[1]);
        assert!(near(hull.verts[11], [-0.721, 0.260, 2.386]), "{:?}", hull.verts[11]);
        assert!(near(hull.verts[4], [-0.617, 1.371, -0.011]), "{:?}", hull.verts[4]);
        // (and a motorcycle's is left as it is)
        let bike = crate::vehicle_model::load_car(&assets, &root.join("resources/vehicle/vp_kaw_ninja_zx14_06")).unwrap();
        let file = bike.hull.as_ref().expect("the bike's hull");
        assert_eq!((file.verts.len(), file.tris.len()), (11, 18));
        let st = bike.simtune.as_ref().unwrap();
        let same = shaped(file, &st.base, 0.3, true);
        assert!(same.verts.iter().zip(&file.verts).all(|(a, b)| (*a - Vec3::from(*b)).length() < 1e-6));
    }

    /// The city's collision file: its size, and the metal strip in front of the garage that no
    /// drawn mesh has - two to three centimetres proud of the road around it.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR containing Complete Edition city resources"]
    fn city_bound_has_the_strip_the_original_drives_over() {
        let root = data();
        let assets = crate::assets::Assets::new(&root);
        let cells = crate::world::city_bound(&assets).expect("the city's bound");
        assert_eq!(cells.len(), 54);
        assert_eq!(cells.iter().map(|c| c.tris.len()).sum::<usize>(), 615_221);
        let mut world = WorldCollider::default();
        for (i, c) in cells.into_iter().enumerate() {
            world.sectors.insert(i, c);
        }
        // (a recorded car's front wheels stand here at the start, on material "metal")
        let at = Vec3::new(819.6121, 17.6063, -808.4069) + Vec3::new(-0.474, 0.0, 0.880) * 1.36;
        let hit = world.ground(at, 1.0, 3.0).expect("ground");
        assert_eq!(hit.surface, Surface::Metal);
        let ahead = world.ground(at + Vec3::new(-0.474, 0.0, 0.880) * 1.2, 1.0, 3.0).expect("ground");
        assert_eq!(ahead.surface, Surface::Road);
    }

    /// A hull driven square into a wall stops within the tick and is not thrown back harder
    /// than the materials' elasticity allows; pushed on against it, it stays put.
    #[test]
    fn a_wall_stops_the_hull_without_a_rattle() {
        let _ = crate::physics::collision::Surface::Wall;
        let mut world = WorldCollider::default();
        // a wall across the way at z = -10, facing +z
        let (a, b, c, d) = (Vec3::new(-20.0, -1.0, -10.0), Vec3::new(20.0, -1.0, -10.0), Vec3::new(20.0, 5.0, -10.0), Vec3::new(-20.0, 5.0, -10.0));
        world.sectors.insert(0, SectorMesh::build_with(vec![[a, b, c], [a, c, d]], vec![Surface::Wall as u8; 2]));
        let verts = vec![
            Vec3::new(-0.9, 0.3, -2.0), Vec3::new(0.9, 0.3, -2.0), Vec3::new(0.9, 1.2, -2.0), Vec3::new(-0.9, 1.2, -2.0),
            Vec3::new(-0.9, 0.3, 2.0), Vec3::new(0.9, 0.3, 2.0), Vec3::new(0.9, 1.2, 2.0), Vec3::new(-0.9, 1.2, 2.0),
        ];
        let tris: Vec<[u16; 3]> = vec![[0, 1, 2], [0, 2, 3], [4, 6, 5], [4, 7, 6], [0, 4, 5], [0, 5, 1], [3, 2, 6], [3, 6, 7], [0, 3, 7], [0, 7, 4], [1, 5, 6], [1, 6, 2]];
        let hull = HullShape::new(verts, &tris);
        let mut st = CarState::new(Vec3::new(0.0, 0.0, -7.8), 0.0);
        st.vel = Vec3::new(0.0, 0.0, -20.0);
        let (mass, inertia, dt) = (1500.0, Vec3::new(2500.0, 3000.0, 800.0), 1.0 / 60.0);
        let touch = |_: Surface, _: Vec3, _: f32| (0.025, 0.125);
        let mut worst = 0.0f32;
        let mut kept = Vec::new();
        for _ in 0..180 {
            // (the engine keeps pushing)
            st.vel.z -= 6.0 * dt;
            st.pos += st.vel * dt;
            if resolve(&mut st, &hull, &world, mass, inertia, Vec3::new(0.0, 0.8, 0.0), dt, touch, None, &mut kept, (Vec3::ZERO, Vec3::ZERO)).is_some() {
                worst = worst.max(st.vel.z);
            }
        }
        // (one contact a polygon: a square hit is taken at a corner and turns the box a little,
        // so its middle comes back faster than the elasticity alone gives)
        assert!(worst < 20.0 * 0.3, "thrown back at {worst} m/s");
        // (along the wall nothing holds it here: no ground, a flank's friction of 0.025)
        // (and held at one corner the box turns on slowly: it is stopped, not at rest)
        assert!(st.vel.length() < 2.0, "still moving at {:?}", st.vel);
        // (it has not gone through, and has not been thrown far back)
        assert!(st.pos.z > -8.05 && st.pos.z < -7.0, "rests at {}", st.pos.z);
    }
}

