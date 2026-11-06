//! Two pieces of the original's mcBrainRacing (decoded 2026-10-21, work/racerai/brain.md 2.7 / 2.8,
//! raceline.md 4.7): the turnaround picker of a slow racer (F-67810 with its arc test
//! F-38160 and the nav mesh's segment test, mesh vfunc 72 = F-87322), and the obstacle
//! selection along the race line (F-86521 -> F-53450 over the object grid 0x8286F1B4).

use crate::aiv::{AivTuning, Obstacle, OBST_FIRST_LEG, OBST_TYPE0};
use crate::brain::Manoeuvre;
use glam::Vec3;
use mcla_formats::rage::navmesh::NavMesh;

/// PowerPC fsel: `a >= 0 ? b : c` (a NaN takes c).
fn fsel(a: f32, b: f32, c: f32) -> f32 {
    if a >= 0.0 { b } else { c }
}

// =================================================================================================
// the nav mesh's segment test (navSpaceMesh vtable 0x820D53C4, slot 72 = F-87322)
// =================================================================================================

/// What F-38160 gives the mesh for its arc test: navActorCapabilities (F-53025 defaults)
/// with +8 min normal-y = -1.0 and +28 width = aiv+25168 (the car's width, 2 x its bound's half
/// extent x), no filter; and navSpaceMesh +76 (height window above a polygon's plane) set to 100
/// for the test (+80, the window below, stays 1.0).
const ABOVE: f32 = 100.0;
const BELOW: f32 = 1.0;
/// F-87322's limits: 128 nodes per list, 201 nodes popped at most.
const LIST: usize = 128;
const MAX_POPS: i32 = 200;

/// A node of the mesh's search graph: a polygon (global index) or a span record (n_polys + record).
type Node = u32;

fn chunk_local(mesh: &NavMesh, poly: u32, p: Vec3) -> Vec3 {
    let r = &mesh.chunk_rows[mesh.polys[poly as usize].chunk as usize];
    let d = p - r[3];
    Vec3::new(r[0].dot(d), r[1].dot(d), r[2].dot(d))
}

/// F-11050 -> F-10756 (vfunc 68, and the point list of vfunc 32): `p` inside the polygon in
/// plan (F-22664: chunk-local, one winding, border included) and between `BELOW` under and
/// `ABOVE` over its plane (h = (n.p + d) / n.y, n.y > 0.001 required).
fn inside(mesh: &NavMesh, poly: u32, p: Vec3) -> bool {
    let (Some(q), Some(pl)) = (mesh.local.get(poly as usize), mesh.polys.get(poly as usize)) else { return false };
    let lp = chunk_local(mesh, poly, p);
    let nv = (pl.nv as usize).min(4);
    let plan = (0..nv).all(|i| {
        let (a, b) = (q.verts[i], q.verts[(i + 1) % nv]);
        (lp.z - a.z) * (b.x - a.x) - (lp.x - a.x) * (b.z - a.z) <= 2e-4
    });
    if !plan || !(q.n.y > 0.001) {
        return false;
    }
    let h = (q.n.y * lp.y + q.n.z * lp.z + q.n.x * lp.x + q.d) / q.n.y;
    h >= -BELOW && h <= ABOVE
}

/// mesh vfunc 24 (F-56129): the ends of the edge from `poly` into a neighbour, (Q, P) as
/// F-77736 takes them. A local link's edge (Q = the vertex the link lists first for this
/// polygon, P the other) is shortened when the car is wider than twice the clearance beyond an end:
/// that end moves in so that half the car's width fits (from Q: w/2 - c_Q; to P: L + c_P - w/2, L
/// the plan length); fails when L < 1e-6 or the two cross. A span record's edge is taken as it is.
fn edge_ends(l: &mcla_formats::rage::navmesh::NavLink, width: f32) -> Option<(Vec3, Vec3)> {
    if l.span != u32::MAX {
        return Some((l.pa, l.pb));
    }
    let (mut v0, mut v1) = (l.pa, l.pb);
    let q = width * width * 0.25;
    let (fa, fb) = (q > l.ca * l.ca, q > l.cb * l.cb);
    if fa || fb {
        let e = v1 - v0;
        let len = (e.z * e.z + e.x * e.x).sqrt();
        if len < 1e-6 {
            return None;
        }
        let hw = width * 0.5;
        let lo = hw - l.ca;
        let hi = len + l.cb - hw;
        if lo >= hi {
            return None;
        }
        let d = e * (1.0 / len);
        if fb {
            v1 = d * hi + v0;
        }
        if fa {
            v0 = d * lo + v0;
        }
    }
    Some((v0, v1))
}

/// F-77736: does the plan segment s + t d (t in 0..1) leave through the edge P -> Q? Only
/// edges crossed one way count (cross(Q - P, d) >= 1e-5). Returns (crossed, beyond): beyond = the
/// segment ends before it reaches the edge's line (its end lies on this side).
#[allow(clippy::too_many_arguments)]
fn crosses(px: f32, pz: f32, qx: f32, qz: f32, sx: f32, sz: f32, dx: f32, dz: f32) -> (bool, bool) {
    let ez = qz - pz;
    let ex = qx - px;
    let cr = ex * dz - ez * dx;
    if cr < 1e-5 {
        return (false, false);
    }
    let s;
    if dx.abs() < 1e-5 {
        let t = (sx - px) / ex;
        if t < 0.0 || t > 1.0 {
            return (false, false);
        }
        s = (t * ez + pz - sz) / dz;
    } else if dz.abs() < 1e-5 {
        let t = (sz - pz) / ez;
        if t < 0.0 || t > 1.0 {
            return (false, false);
        }
        s = (t * ex + px - sx) / dx;
    } else {
        let t = ((sx - px) * dz + (pz - sz) * dx) / cr;
        if t < 0.0 || t > 1.0 {
            return (false, false);
        }
        s = (t * ex + px - sx) / dx;
    }
    if s > 1.0 {
        return (false, true);
    }
    (s >= 0.0, false)
}

/// mesh vfunc 28 (F-32987) for one node: the neighbours the segment a -> b passes into (in the
/// iterator's order, then sorted and made unique as F-15132 does), and whether the segment ends
/// inside this node. Neighbours (navSpaceMeshNeighborIterator): a polygon's local links that pass the
/// width test (w^2 <= 1e-12 or (c_a + L + c_b)^2 >= w^2, L the edge's plan length; the min normal-y
/// of -1 passes all), then its span records (no test); a span record's two polygons (A, then B).
/// A neighbour whose edge vfunc 24 cannot give (a span node's, or a shortened edge that vanished) is
/// taken as crossed.
fn step(mesh: &NavMesh, node: Node, a: Vec3, b: Vec3, width: f32, out: &mut Vec<Node>) -> bool {
    out.clear();
    let n = mesh.polys.len() as u32;
    let d = b - a;
    let mut ends = false;
    if node < n {
        for l in mesh.links_of(node) {
            let nb = if l.span == u32::MAX {
                let w2 = width * width;
                if w2 > 1e-12 {
                    let len = ((l.a.x - l.b.x).powi(2) + (l.a.z - l.b.z).powi(2)).sqrt();
                    let span = l.ca + len + l.cb;
                    if span * span < w2 {
                        continue;
                    }
                }
                l.poly
            } else {
                n + l.span
            };
            match edge_ends(l, width) {
                Some((q, p)) => {
                    let (hit, beyond) = crosses(p.x, p.z, q.x, q.z, a.x, a.z, d.x, d.z);
                    if hit {
                        out.push(nb);
                    } else if beyond {
                        ends = true;
                    }
                }
                None => out.push(nb),
            }
        }
    } else if let Some(s) = mesh.span_recs.get((node - n) as usize) {
        out.push(s.pa);
        out.push(s.pb);
    }
    out.sort_unstable();
    out.dedup();
    ends
}

/// vfunc 68 (F-85002): `p` inside the polygon node (span nodes: never).
fn node_has(mesh: &NavMesh, node: Node, p: Vec3) -> bool {
    (node as usize) < mesh.polys.len() && inside(mesh, node, p)
}

/// The segment test F-38160 runs (mesh vfunc 72 = F-87322, filter off, the "use filter"
/// flag 0): can a car of width `width` go straight from `a` to `b` over the mesh? A breadth-first
/// walk from the polygons that contain `a` (vfunc 32, at most 128, ascending): each popped node (the
/// last of the list first) is kept in the generation's done list (up to 128); if the segment ends in
/// it and `b` lies in it -> true; else the neighbours it passes into join the next generation (128 at
/// most) unless already listed in this or the next generation or done in this or the last one. More
/// than 201 nodes popped -> false. When nothing is left: true if `b` lies in a node done in the
/// last generation, or in the one before it.
pub fn segment_on_mesh(mesh: &NavMesh, a: Vec3, b: Vec3, width: f32) -> bool {
    // vfunc 32 (F-15418 -> F-43289 -> F-49070): the point's polygons, sorted
    let starts = mesh.sphere_query(a, 0.0, 1024, |i, _| inside(mesh, i, a));
    let mut open: [Vec<Node>; 2] = [starts.into_iter().take(LIST).collect(), Vec::new()];
    let mut done: [Vec<Node>; 2] = [Vec::new(), Vec::new()];
    let (mut cur, mut other) = (0usize, 1usize);
    // (r14 / [r1+80] of F-87322: the generation before and the current one)
    let (mut prev_gen, mut this_gen) = (1usize, 0usize);
    let mut pops = 0i32;
    let mut nbs = Vec::new();
    if !open[cur].is_empty() {
        loop {
            done[cur].clear();
            while let Some(node) = open[cur].pop() {
                if pops > MAX_POPS {
                    return false;
                }
                pops += 1;
                if done[cur].len() != LIST {
                    done[cur].push(node);
                }
                if step(mesh, node, a, b, width, &mut nbs) && node_has(mesh, node, b) {
                    return true;
                }
                for &nb in &nbs {
                    if open[other].len() == LIST {
                        break;
                    }
                    if open[other].contains(&nb) || open[cur].contains(&nb) || done[other].contains(&nb) || done[cur].contains(&nb) {
                        continue;
                    }
                    open[other].push(nb);
                }
            }
            std::mem::swap(&mut cur, &mut other);
            std::mem::swap(&mut prev_gen, &mut this_gen);
            if open[cur].is_empty() {
                break;
            }
        }
    }
    for g in [prev_gen, this_gen] {
        if done[g].iter().rev().any(|&n| node_has(mesh, n, b)) {
            return true;
        }
    }
    false
}

// =================================================================================================
// the turnaround picker (F-67810) and its arc test (F-38160)
// =================================================================================================

/// What the arc test reads of the car.
#[derive(Debug, Clone, Copy)]
pub struct TurnCar {
    /// the car matrix (F-35658): +48 position, +0 row 0 (right), +32 row 2 (= -forward)
    pub pos: Vec3,
    pub right: Vec3,
    pub back: Vec3,
    /// aiv+25168: 2 x the car's bound half extents (F-98453(car+8)+16); x = width, z = length
    pub size: Vec3,
    /// the chassis' wheels as it lists them (chassis +412, +416, +420, +424): each wheel's +64
    /// position, world
    pub wheels: [Vec3; 4],
    /// chassis +24 (2 = a motorcycle)
    pub wheel_count: usize,
    /// chassis +372: which wheels the engine turns now (vehsim `drive`: 0 rear, 1 front, 2 / 3 all,
    /// 4 none)
    pub drive: u32,
}

/// F-38160(angle, reverse, pivot): can the car turn by `angle` (rad, + towards the left = the
/// sign of F-32060) over the mesh? A point Q is swung about a centre C in 4 steps of angle/4
/// (Q rotated about y: x' = x cos + z sin, z' = z cos - x sin), each step's segment tested with the
/// mesh (vfunc 72, width aiv+25168, navmesh +76 = 100 during the test). s = sign(angle) (0 for 0).
/// - pivot: C = a wheel's position: wheel slot (2 wheels: +412 if the drive is 0 or 2, else +416;
///   else drive 0 / 2: angle <= 0 -> +412, > 0 -> +420; other drives: angle >= 0 -> +416, < 0 ->
///   +424); Q = car + back x length x (drive 0 / 2 ? 0.5 : -0.5) - right x 0.5 s width. The
///   `reverse` argument is not read.
/// - circle: radius R = (brain +336 ? 1.25 : 1) / MaxCurvature (aiv tuning +8); Q = car + back x
///   length x (reverse ? 0.5 : -0.5); C = Q - right x s R x (reverse ? -1 : 1).
/// `backing_up` = brain +336 as the call finds it.
pub fn arc_clear(mesh: &NavMesh, car: &TurnCar, backing_up: bool, max_curvature: f32, angle: f32, reverse: bool, pivot: bool) -> bool {
    let (c, off);
    if pivot {
        let rear = car.drive == 0 || car.drive == 2;
        let slot = if car.wheel_count == 2 {
            if rear { 0 } else { 1 }
        } else if rear {
            if angle <= 0.0 { 0 } else { 2 }
        } else if angle >= 0.0 {
            1
        } else {
            3
        };
        c = car.wheels[slot];
        let k = if rear { 0.5 } else { -0.5 };
        let s = fsel(angle, fsel(-angle, 0.0, 1.0), -1.0);
        off = (car.pos - c) + car.back * (car.size.z * k) + car.right * (s * car.size.x * -0.5);
    } else {
        let r = if backing_up { 1.25 } else { 1.0 } / max_curvature;
        let dir = if reverse { -1.0 } else { 1.0 };
        let s = fsel(angle, fsel(-angle, 0.0, 1.0), -1.0);
        off = car.right * (s * dir * r);
        let k = if reverse { 0.5 } else { -0.5 };
        c = (car.pos - off) + car.back * (car.size.z * k);
    }
    let mut prev = c + off;
    for i in 1..=4 {
        let phi = i as f32 * angle * 0.25;
        let (sn, cs) = phi.sin_cos();
        let v = Vec3::new(off.z * sn + off.x * cs, off.y, off.z * cs - off.x * sn);
        let p = v + c;
        if !segment_on_mesh(mesh, prev, p, car.size.x) {
            return false;
        }
        prev = p;
    }
    true
}

/// F-67810: the turnaround picker, run while the stuck detector is idle (+340 == 0).
/// `angle_line` = F-32060(forward, next line point - car) (F-28749), `angle_corridor` the
/// same to the corridor point ahead (F-43876, read only when +332 is 0 and |angle_line| > 135
/// degrees); `speed` = car +224; `avoid_handbrake` = +338.
///
/// Faster than 10 m/s (0x82003A8C): +336 = +332 = 0. Else, with a = angle_line: handbrake turns are
/// allowed when +337 is set or (+338 is clear and +332 is 0); +332 = 0 is chosen by the sign
/// (a >= 0 -> 1, else 2; past 135 degrees by the corridor's sign). Then the first arc that clears
/// (a+ = a or a + 2 pi, a- = a or a - 2 pi; for +332 == 2 the same with the sides swapped):
/// 1. pivot a+ (only if allowed first) -> turn 1, handbrake; 2. circle a+ -> turn 1;
/// 3. pivot a+ (if not allowed first) -> turn 1, handbrake; 4. pivot a- -> turn 2, handbrake;
/// 5. circle a- -> turn 2; 6. circle a+ backing up -> +336 = 1, turn 1 if it clears, else 0.
/// +336 = 0 for 1..5, +337 = 0 for 2, 5, 6. Last: when not backing up, |a| < 90 degrees and +332 is
/// the side of a, +332 = 0 (+337 is left).
#[allow(clippy::too_many_arguments)]
pub fn turnaround_car(m: &mut Manoeuvre, avoid_handbrake: bool, angle_line: f32, angle_corridor: f32, speed: f32, car: &TurnCar, max_curvature: f32, mesh: &NavMesh) {
    if speed > 10.0 {
        m.reverse = false;
        m.turn = 0;
        return;
    }
    let a = angle_line;
    let allowed = m.handbrake || (!avoid_handbrake && m.turn == 0);
    if m.turn == 0 {
        let b = if a.abs() > 2.356_194_5 { angle_corridor } else { a };
        m.turn = if b >= 0.0 { 1 } else { 2 };
    }
    let two_pi = 6.283_185_5f32;
    let pos = fsel(a, a, a + two_pi);
    let neg = fsel(-a, a, a - two_pi);
    let backing = m.reverse;
    let test = |ang: f32, rev: bool, pivot: bool| arc_clear(mesh, car, backing, max_curvature, ang, rev, pivot);
    // (same side first, then the other; the last resort backs up)
    let (same, other, side, other_side) = if m.turn == 1 { (pos, neg, 1u8, 2u8) } else { (neg, pos, 2u8, 1u8) };
    let set = |m: &mut Manoeuvre, turn: u8, hb: bool| {
        m.turn = turn;
        m.reverse = false;
        m.handbrake = hb;
    };
    if allowed && test(same, false, true) {
        set(m, side, true);
    } else if test(same, false, false) {
        set(m, side, false);
    } else if !allowed && test(same, false, true) {
        set(m, side, true);
    } else if test(other, false, true) {
        set(m, other_side, true);
    } else if test(other, false, false) {
        set(m, other_side, false);
    } else {
        let ok = test(same, true, false);
        m.reverse = true;
        m.handbrake = false;
        m.turn = if ok { side } else { 0 };
    }
    if !m.reverse && a.abs() < 1.570_796_4 && m.turn == (if a >= 0.0 { 1 } else { 2 }) {
        m.turn = 0;
    }
}

/// The picker with what brain.rs passes today.
/// (approx.: the car's width is taken as the aiv's default 2.0 (aiv+25168 before the car is set),
/// the drive as rear (0), and the wheels as the corners of the car's length x width box in the slot
/// order (+412 front right, +416 rear left, +420 front left, +424 rear right) - the chassis' slot
/// order is not decoded; switch the caller to [`turnaround_car`] with the chassis' own values)
#[allow(clippy::too_many_arguments)]
pub fn turnaround(m: &mut Manoeuvre, avoid_handbrake: bool, angle_line: f32, angle_corridor: f32, speed: f32, pos: Vec3, forward: Vec3, right: Vec3, car_length: f32, max_curvature: f32, mesh: &NavMesh) {
    let w = 2.0;
    let (f, r) = (forward * (car_length * 0.5), right * (w * 0.5));
    let car = TurnCar {
        pos,
        right,
        back: -forward,
        size: Vec3::new(w, 1.5, car_length),
        wheels: [pos + f + r, pos - f - r, pos + f - r, pos - f + r],
        wheel_count: 4,
        drive: 0,
    };
    turnaround_car(m, avoid_handbrake, angle_line, angle_corridor, speed, &car, max_curvature, mesh);
}

// =================================================================================================
// obstacles (F-86521 -> F-53450)
// =================================================================================================

/// mcObstacle +6, the kinds filed in the object grid 0x8286F1B4 (set by the root constructor
/// F-99762): 0 a player's car (mcPlayer, F-96799), 1 a computer racer (mcAiRacer,
/// F-12035), 2 an ambient car (mcAmbientObstacleCar, F-60800, filed by F-27899),
/// 3 a prop (mcProp, F-13009; filed by F-47614 only while its physics state +128 is 2),
/// 4 a pedestrian's (mcAmbientObstaclePed, F-87033). The brain always ignores kind 4.
pub const KIND_PLAYER: u8 = 0;
pub const KIND_RACER: u8 = 1;
pub const KIND_AMBIENT_CAR: u8 = 2;
pub const KIND_PROP: u8 = 3;
pub const KIND_PED: u8 = 4;

/// One object of the grid as F-53450 reads it through its vtable.
#[derive(Debug, Clone, Copy)]
pub struct ObjectInfo {
    /// vfunc 1 -> matrix +48. Players / racers: the car's matrix (F-35658); ambient cars: their
    /// instance's matrix; props: the prop instance's (+16)
    pub pos: Vec3,
    /// vfunc 2. Players / racers: the car's collider linear velocity (chassis +128 -> +272; the
    /// racer's +432 without a car); ambient cars: entity +208; props: 0 (0x828608E0). Its y is
    /// dropped here.
    pub vel: Vec3,
    /// -(matrix row 2): the object's forward
    pub forward: Vec3,
    /// vfunc 3: the full (width, length). Players / racers: mcRacer +472 / +476 = 2 x max(|max.x|,
    /// |min.x|), 2 x max(|max.z|, |min.z|) of the car's bound box (F-98453(car+8) +16 / +32,
    /// F-57218); ambient cars and peds: the bound box's max - min in x and z (no bound: 3.0 and
    /// 3e32); props: archetype +336 / +340
    pub extent: (f32, f32),
    /// mcObstacle +6 (KIND_*)
    pub kind: u8,
}

impl ObjectInfo {
    /// The port's car as the grid object it stands for. Players and racers: the racer extent F-57218
    /// (`oppmgr::racer_extent`) of the car's composite bound box (`CarParams::composite_box`, the box
    /// F-98453(car + 8) answers); ambient cars: their type's bound box max - min (`traffic::bound_box`).
    /// (approx.: the original measures the box once, when the car is made, with the wheel parts where
    /// the suspension then holds them - the live box moves by ~1e-4 m from run to run; police cars are
    /// taken as ambient cars, without a type box: 2 x the tuning box's half extents; racers = cars with
    /// a brain or pinned to the event with an AI driver)
    pub fn from_car(_index: usize, c: &crate::vehicles::CarInstance) -> ObjectInfo {
        let kind = if c.is_player {
            KIND_PLAYER
        } else if !c.cop && c.ai.as_ref().map_or(false, |a| a.brain.is_some() || c.pinned || c.racer) {
            KIND_RACER
        } else {
            KIND_AMBIENT_CAR
        };
        let extent = match (kind, c.model.bound_box) {
            (KIND_AMBIENT_CAR, Some((mn, mx))) => (mx.x - mn.x, mx.z - mn.z),
            (KIND_AMBIENT_CAR, None) => {
                let h = c.params().body_half;
                (h.x * 2.0, h.z * 2.0)
            }
            _ => {
                let (lo, hi) = c.params().composite_box();
                let e = crate::oppmgr::racer_extent(lo.to_array(), hi.to_array());
                (e.width, e.length)
            }
        };
        ObjectInfo { pos: c.state.pos, vel: c.state.vel, forward: c.state.forward(), extent, kind }
    }
}

/// The object grid (0x8286F1B4, ctor F-87945, set up by F-38432): 40 m cells, x by z.
#[derive(Debug, Clone, Copy)]
pub struct ObjectGrid {
    pub min: [f32; 2],
    pub n: [i32; 2],
    /// false: no bounds (every point inside; cells at multiples of 40 m, as the bounded grid's are)
    pub bounded: bool,
}

const CELL: f32 = 40.0;
static CITY_GRID: std::sync::OnceLock<ObjectGrid> = std::sync::OnceLock::new();

impl ObjectGrid {
    /// F-38432 from the level's bounds (min, max; x and z used): the box widened by 20 m along
    /// its diagonal each way, then out to whole 40 m cells from the origin.
    pub fn from_bounds(min: Vec3, max: Vec3) -> ObjectGrid {
        let (mut ax, mut az, mut bx, mut bz) = (min.x, min.z, max.x, max.z);
        let (dx, dz) = (bx - ax, bz - az);
        let l2 = dz * dz + dx * dx;
        let inv = fsel(-l2, 0.0, 1.0 / l2.sqrt());
        let (px, pz) = (inv * dx * 20.0, dz * inv * 20.0);
        bx += px;
        bz += pz;
        ax -= px;
        az -= pz;
        let lo = |v: f32| if v >= 0.0 { 0 } else { 1 - (v * 0.025) as i32 };
        let hi = |v: f32| if v <= 0.0 { 0 } else { 1 - (v * -0.025) as i32 };
        let (r7, r11, r8, r10) = (lo(ax), hi(bx), lo(az), hi(bz));
        ObjectGrid { min: [r7 as f32 * -40.0, r8 as f32 * -40.0], n: [(r11 + r7).max(1), (r10 + r8).max(1)], bounded: true }
    }
    pub fn unbounded() -> ObjectGrid {
        ObjectGrid { min: [0.0; 2], n: [0; 2], bounded: false }
    }
    /// The city's grid: F-38432's caller (the city layer's set-up, F-67451) hands it the night-ambient map's extents,
    /// the object's +0 / +16 vectors = `m_ext_Min` / `m_ext_Max` of `tune/city/sc/grid.aogrid` (the object read from
    /// the running original: formats aomap.rs `grid_parse_matches_the_original`). Unbounded until `init_city` ran.
    pub fn city() -> ObjectGrid {
        CITY_GRID.get().copied().unwrap_or_else(ObjectGrid::unbounded)
    }
    /// Read the city's extents once (`<data>/tune/city/sc`).
    pub fn init_city(data_dir: &str) {
        CITY_GRID.get_or_init(|| match mcla_formats::rage::aomap::AoMap::load(&std::path::Path::new(data_dir).join("tune/city/sc")) {
            Some(m) => ObjectGrid::from_bounds(m.min, m.max),
            None => ObjectGrid::unbounded(),
        });
    }
    /// F-59709: the cell (ix, iz) of a point, `None` outside (the original then goes on with
    /// ix = iz = 0 for the neighbours).
    fn cell(&self, x: f32, z: f32) -> Option<(i32, i32)> {
        if !self.bounded {
            return Some(((x * 0.025).floor() as i32, (z * 0.025).floor() as i32));
        }
        let max = [self.n[0] as f32 * CELL + self.min[0], self.n[1] as f32 * CELL + self.min[1]];
        if x < self.min[0] || x >= max[0] || z < self.min[1] || z >= max[1] {
            return None;
        }
        Some((((x - self.min[0]) * 0.025) as i32, ((z - self.min[1]) * 0.025) as i32))
    }
    /// The cell's key (the u16 cell number iz nx + ix of a bounded grid), `None` off the grid
    /// (F-74216 / F-37308).
    fn key(&self, ix: i32, iz: i32) -> Option<i64> {
        if !self.bounded {
            return Some(((ix as i64) << 32) | (iz as u32 as i64));
        }
        if ix < 0 || ix >= self.n[0] || iz < 0 || iz >= self.n[1] {
            return None;
        }
        let id = ((iz * self.n[0] + ix) as u32 & 0xFFFF) as i64;
        (id < (self.n[0] as i64 * self.n[1] as i64)).then_some(id)
    }
}

/// What F-86521 reads of the brain's own car.
#[derive(Debug, Clone, Copy)]
pub struct SelCar {
    /// car matrix +48 and -(row 2)
    pub pos: Vec3,
    pub forward: Vec3,
    /// the car's collider linear velocity (chassis +128 -> +272)
    pub vel: Vec3,
    /// car +224
    pub speed: f32,
    /// F-98453(car+8) +24: the bound box's max z (half the length for a centred box)
    pub half_length: f32,
}

/// F-86521 -> F-53450: the obstacles the brain hands the aiv (F-69761 then
/// F-70624 for each), in order.
///
/// `ignore` = brain +408 (bit k ignores kind k); bit 4 is set first, always. Car direction D =
/// the velocity's direction when |vel|^2 > 1 (F-41334), else the forward; its speed V = vel.D.
/// Walking the aiv's control points from the first (the ring at aiv+25440): for each point P, the
/// grid cell under it, then (ix-1, iz-1), (ix-1, iz), (ix-1, iz+1), (ix, iz-1), (ix, iz+1),
/// (ix+1, iz-1), (ix+1, iz), (ix+1, iz+1); per cell its objects (stopping at 32): not the brain's own
/// racer, kind not ignored; with d = (pos - car).D, v = vel_xz.D and R = 0.5 max(extent) +
/// half_length: dropped when d < -R, or v > V and d > R (faster and clear ahead); dropped when
/// |pos.y - P.y| > MaxObstacleHeightDelta or the plan distance to P > MaxObstacleFlatDelta; once only.
/// Kept with direction -(forward_xz normalised; (1, 0, 0) when |forward_xz|^2 <= 1e-5), flag 0x80
/// when v > V - 10, 0x40 for kind 0. After each point t += |P - car| / max(speed, 10); past
/// ObstacleLookaheadTime the walk stops.
/// (approx.: within a cell the original's list is in the order objects last moved into it, newest
/// first; here the slice's order. An object's cell is taken from `pos`, the original's from where it
/// was at its own last update)
#[allow(clippy::too_many_arguments)]
pub fn select_obstacles_car(me: usize, objects: &[ObjectInfo], line: &[Vec3], car: &SelCar, ignore: u32, grid: &ObjectGrid, tuning: &AivTuning) -> Vec<Obstacle> {
    let ignore = ignore | 16;
    let mut out: Vec<Obstacle> = Vec::new();
    let mut taken: Vec<usize> = Vec::new();
    let fma = |a: f32, b: f32, c: f32| (a as f64).mul_add(b as f64, c as f64) as f32;
    // (F-41334 with eps 1.0 and the forward as fallback; V = fma(vel.x, D.x, fma(vel.y, D.y, vel.z D.z)))
    let dir = crate::aiv::norm_or(car.vel, 1.0, car.forward);
    let v_car = fma(car.vel.x, dir.x, fma(car.vel.y, dir.y, car.vel.z * dir.z));
    let inv = 1.0 / car.speed.max(10.0);
    // objects by cell key
    let mut cells: hashbrown::HashMap<i64, Vec<usize>> = hashbrown::HashMap::new();
    for (j, o) in objects.iter().enumerate() {
        if let Some(k) = grid.cell(o.pos.x, o.pos.z).and_then(|(x, z)| grid.key(x, z)) {
            cells.entry(k).or_default().push(j);
        }
    }
    let mut t = 0.0f32;
    for &p in line {
        let (ix, iz, centre) = match grid.cell(p.x, p.z) {
            Some((x, z)) => (x, z, grid.key(x, z)),
            None => (0, 0, None),
        };
        let mut keys = [centre; 9];
        for (k, (dx, dz)) in [(-1, -1), (-1, 0), (-1, 1), (0, -1), (0, 1), (1, -1), (1, 0), (1, 1)].into_iter().enumerate() {
            keys[k + 1] = grid.key(ix + dx, iz + dz);
        }
        for key in keys.into_iter().flatten() {
            if let Some(list) = cells.get(&key) {
                select_cell(list, me, objects, ignore, car.pos, dir, v_car, car.half_length, p, tuning, 0, &mut taken, &mut out);
            }
        }
        // (t = fma(sqrt(fma(dx, dx, fma(dz, dz, dy dy))), 1 / max(speed, 10), t), d = car - P)
        let q = car.pos - p;
        t = fma(fma(q.x, q.x, fma(q.z, q.z, q.y * q.y)).sqrt(), inv, t);
        if t > tuning.obstacle_lookahead_time {
            break;
        }
    }
    out
}

/// F-53450 per cell: one grid cell's object list, in the list's order, as F-53450 walks it (the caller's
/// direction D and speed V, the brain's half length, the line point P). `base` = the obstacles the aiv held before
/// the selection (aiv +25104; the walk stops when base + out reaches 32). Products as the original's: d =
/// fma(D.y, r.y, fma(D.z, r.z, D.x r.x)), v = fma(D.x, v.x, D.z v.z), R = fma(max(w, l), 0.5, half length), the plan
/// distance fma(q.z, q.z, q.x q.x) against the rounded square of MaxObstacleFlatDelta; the direction 0 - n, n =
/// (-forward_xz) times the vector unit's refined reciprocal root of its squared length (z + (y + x)) when that is >
/// 1e-5, else (1, 0, 0).
#[allow(clippy::too_many_arguments)]
pub fn select_cell(list: &[usize], me: usize, objects: &[ObjectInfo], ignore: u32, car_pos: Vec3, dir: Vec3, v_car: f32, half_length: f32, p: Vec3, tuning: &AivTuning, base: usize, taken: &mut Vec<usize>, out: &mut Vec<Obstacle>) {
    let fma = |a: f32, b: f32, c: f32| (a as f64).mul_add(b as f64, c as f64) as f32;
    for &j in list {
        if base + out.len() >= 32 {
            return;
        }
        let o = &objects[j];
        if j == me || (o.kind < 32 && ignore >> o.kind & 1 != 0) {
            continue;
        }
        let rel = o.pos - car_pos;
        let d = fma(dir.y, rel.y, fma(dir.z, rel.z, dir.x * rel.x));
        let v = fma(dir.y, 0.0, fma(dir.x, o.vel.x, dir.z * o.vel.z));
        let r = fma(fsel(o.extent.0 - o.extent.1, o.extent.0, o.extent.1), 0.5, half_length);
        if d < -r || (v > v_car && d > r) {
            continue;
        }
        let first_leg = v > v_car - 10.0;
        let q = o.pos - p;
        let fd = tuning.max_obstacle_flat_delta;
        if q.y.abs() > tuning.max_obstacle_height_delta || fma(q.z, q.z, q.x * q.x) > fd * fd {
            continue;
        }
        if taken.contains(&j) {
            continue;
        }
        taken.push(j);
        let f = Vec3::new(o.forward.x, 0.0, o.forward.z);
        let l2 = f.z * f.z + (f.y * f.y + f.x * f.x);
        let n = if l2 > 1e-5 {
            let k = crate::aiv::vrsqrt1(l2);
            Vec3::new(f.x * k, f.y * k, f.z * k)
        } else {
            Vec3::X
        };
        let flags = if first_leg { OBST_FIRST_LEG } else { 0 } | if o.kind == KIND_PLAYER { OBST_TYPE0 } else { 0 };
        out.push(Obstacle { pos: o.pos, vel: Vec3::new(o.vel.x, 0.0, o.vel.z), dir: Vec3::ZERO - n, extent: o.extent, flags });
    }
}

/// The selection with what brain.rs passes today.
/// (approx.: the car's velocity is taken as forward x speed, its half length as the aiv's default
/// 2.5, no kinds ignored but the pedestrians', and the grid without bounds)
#[allow(clippy::too_many_arguments)]
pub fn select_obstacles(me: usize, objects: &[ObjectInfo], line: &[Vec3], car_pos: Vec3, car_fwd: Vec3, car_speed: f32, tuning: &AivTuning) -> Vec<Obstacle> {
    let car = SelCar { pos: car_pos, forward: car_fwd, vel: car_fwd * car_speed, speed: car_speed, half_length: 2.5 };
    select_obstacles_car(me, objects, line, &car, 0, &ObjectGrid::unbounded(), tuning)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mcla_formats::rage::navmesh::{NavLink, NavPoly, PolyLocal};

    /// A flat mesh of `nx` x `nz` squares of `size` m from `origin`, minus `holes`; local links with
    /// clearance `cl` beyond both ends. Winding (x0,z0), (x0,z1), (x1,z1), (x1,z0) (inside for
    /// F-22664); a link leaving through the edge a -> b lists b first (pa) as the file does.
    fn grid_mesh(nx: i32, nz: i32, size: f32, origin: Vec3, holes: &[(i32, i32)], cl: f32) -> NavMesh {
        let mut m = NavMesh::default();
        let idx = |i: i32, j: i32| -> Option<u32> {
            if i < 0 || j < 0 || i >= nx || j >= nz || holes.contains(&(i, j)) {
                return None;
            }
            let mut k = 0u32;
            for jj in 0..nz {
                for ii in 0..nx {
                    if holes.contains(&(ii, jj)) {
                        continue;
                    }
                    if (ii, jj) == (i, j) {
                        return Some(k);
                    }
                    k += 1;
                }
            }
            None
        };
        m.chunk_rows = vec![[Vec3::X, Vec3::Y, Vec3::Z, Vec3::ZERO]];
        m.chunk_first = vec![0];
        m.quads = vec![None];
        for j in 0..nz {
            for i in 0..nx {
                if idx(i, j).is_none() {
                    continue;
                }
                let (x0, z0) = (origin.x + i as f32 * size, origin.z + j as f32 * size);
                let (x1, z1) = (x0 + size, z0 + size);
                let v = [Vec3::new(x0, 0.0, z0), Vec3::new(x0, 0.0, z1), Vec3::new(x1, 0.0, z1), Vec3::new(x1, 0.0, z0)];
                let first = m.links.len() as u32;
                // edges 0..3 lead to (i-1, j), (i, j+1), (i+1, j), (i, j-1)
                for (e, (di, dj)) in [(-1, 0), (0, 1), (1, 0), (0, -1)].into_iter().enumerate() {
                    if let Some(nb) = idx(i + di, j + dj) {
                        let (a, b) = (v[e], v[(e + 1) % 4]);
                        m.links.push(NavLink { poly: nb, a, b, len: size, pa: b, pb: a, ca: cl, cb: cl, span: u32::MAX });
                    }
                }
                m.polys.push(NavPoly {
                    chunk: 0,
                    material: 0,
                    flags: 2,
                    area: size * size,
                    center: (v[0] + v[2]) * 0.5,
                    normal: Vec3::Y,
                    nv: 4,
                    verts: v,
                    first_link: first,
                    n_links: (m.links.len() as u32 - first) as u8,
                });
                m.local.push(PolyLocal { verts: v, n: Vec3::Y, d: 0.0 });
            }
        }
        m
    }

    fn open_mesh() -> NavMesh {
        grid_mesh(20, 20, 10.0, Vec3::new(-100.0, 0.0, -100.0), &[], 100.0)
    }

    #[test]
    fn segment_test() {
        let m = open_mesh();
        // inside one polygon, across several, off the mesh
        assert!(segment_on_mesh(&m, Vec3::new(1.0, 0.5, 1.0), Vec3::new(3.0, 0.5, 4.0), 2.0));
        assert!(segment_on_mesh(&m, Vec3::new(-55.0, 0.5, -42.0), Vec3::new(37.0, 0.5, 61.0), 2.0));
        assert!(segment_on_mesh(&m, Vec3::new(5.0, 0.5, 5.0), Vec3::new(-15.0, 0.5, 5.0), 2.0));
        assert!(!segment_on_mesh(&m, Vec3::new(95.0, 0.5, 5.0), Vec3::new(105.0, 0.5, 5.0), 2.0));
        assert!(!segment_on_mesh(&m, Vec3::new(150.0, 0.5, 5.0), Vec3::new(151.0, 0.5, 5.0), 2.0));
        // up to 100 m above the plane and 1 m below it
        assert!(segment_on_mesh(&m, Vec3::new(1.0, 99.0, 1.0), Vec3::new(2.0, 99.0, 1.0), 2.0));
        assert!(!segment_on_mesh(&m, Vec3::new(1.0, -1.5, 1.0), Vec3::new(2.0, -1.5, 1.0), 2.0));
        // a hole in the way
        let h = grid_mesh(5, 1, 10.0, Vec3::ZERO, &[(2, 0)], 100.0);
        assert!(!segment_on_mesh(&h, Vec3::new(5.0, 0.0, 5.0), Vec3::new(45.0, 0.0, 5.0), 2.0));
        assert!(segment_on_mesh(&h, Vec3::new(5.0, 0.0, 5.0), Vec3::new(15.0, 0.0, 6.0), 2.0));
        // a narrow way: no clearance beyond a 10 m edge, the shortened edge (w/2 in from each end)
        let n = grid_mesh(1, 3, 10.0, Vec3::ZERO, &[], 0.0);
        assert!(segment_on_mesh(&n, Vec3::new(5.0, 0.0, 5.0), Vec3::new(5.0, 0.0, 25.0), 2.0));
        assert!(!segment_on_mesh(&n, Vec3::new(0.5, 0.0, 5.0), Vec3::new(0.5, 0.0, 25.0), 2.0));
        // wider than the edge and its clearances: the link is not followed
        assert!(!segment_on_mesh(&n, Vec3::new(5.0, 0.0, 5.0), Vec3::new(5.0, 0.0, 25.0), 11.0));
    }

    #[test]
    fn crossing_rule() {
        // leaving through P (0,10) -> Q (10,10) going +z
        assert_eq!(crosses(0.0, 10.0, 10.0, 10.0, 5.0, 5.0, 0.0, 10.0), (true, false));
        // ends before the edge
        assert_eq!(crosses(0.0, 10.0, 10.0, 10.0, 5.0, 5.0, 0.0, 2.0), (false, true));
        // the other way round does not count
        assert_eq!(crosses(10.0, 10.0, 0.0, 10.0, 5.0, 5.0, 0.0, 10.0), (false, false));
        // past the edge's end
        assert_eq!(crosses(0.0, 10.0, 10.0, 10.0, 15.0, 5.0, 0.0, 10.0), (false, false));
    }

    fn car_at(pos: Vec3) -> TurnCar {
        // facing -z, right +x; 2 x 4.5 m
        let (w, l) = (2.0, 4.5);
        let (f, r) = (Vec3::new(0.0, 0.0, -l * 0.4), Vec3::new(w * 0.45, 0.0, 0.0));
        TurnCar {
            pos,
            right: Vec3::X,
            back: Vec3::Z,
            size: Vec3::new(w, 1.4, l),
            wheels: [pos + f + r, pos - f - r, pos + f - r, pos - f + r],
            wheel_count: 4,
            drive: 0,
        }
    }

    #[test]
    fn turnaround_open() {
        let m = open_mesh();
        let car = car_at(Vec3::new(0.0, 0.3, 0.0));
        // too fast: nothing
        let mut man = Manoeuvre { reverse: true, turn: 2, handbrake: false };
        turnaround_car(&mut man, false, 2.0, 2.0, 12.0, &car, 0.2, &m);
        assert_eq!((man.reverse, man.turn), (false, 0));
        // the line behind on the left: a handbrake turn about the wheel, kept (|a| > 90 deg)
        let mut man = Manoeuvre::default();
        turnaround_car(&mut man, false, 2.5, 2.5, 3.0, &car, 0.2, &m);
        assert_eq!(man, Manoeuvre { reverse: false, turn: 1, handbrake: true });
        // to the right
        let mut man = Manoeuvre::default();
        turnaround_car(&mut man, false, -2.5, -2.5, 3.0, &car, 0.2, &m);
        assert_eq!(man, Manoeuvre { reverse: false, turn: 2, handbrake: true });
        // handbrake turns avoided: the circle (radius 1 / MaxCurvature)
        let mut man = Manoeuvre { reverse: false, turn: 1, handbrake: false };
        turnaround_car(&mut man, true, 2.5, 2.5, 3.0, &car, 0.2, &m);
        assert_eq!(man, Manoeuvre { reverse: false, turn: 1, handbrake: false });
        // within 90 degrees on the chosen side: the turn is cancelled, the handbrake flag left
        let mut man = Manoeuvre::default();
        turnaround_car(&mut man, false, 0.5, 0.5, 3.0, &car, 0.2, &m);
        assert_eq!(man, Manoeuvre { reverse: false, turn: 0, handbrake: true });
        // past 135 degrees the corridor's side picks the way
        let mut man = Manoeuvre::default();
        turnaround_car(&mut man, false, 2.8, -1.0, 3.0, &car, 0.2, &m);
        assert_eq!(man.turn, 2);
    }

    #[test]
    fn turnaround_blocked() {
        // a corridor 6 m wide along z: no arc of a 5 m circle or the wheel pivot fits; nothing at
        // all -> backing up, turn 0
        let m = grid_mesh(1, 20, 6.0, Vec3::new(-3.0, 0.0, -60.0), &[], 0.0);
        let car = car_at(Vec3::new(0.0, 0.3, 0.0));
        let mut man = Manoeuvre::default();
        turnaround_car(&mut man, false, 3.0, 3.0, 1.0, &car, 0.2, &m);
        assert!(man.reverse && !man.handbrake);
        assert_eq!(man.turn, 0);
        // off the mesh: every test fails
        let far = car_at(Vec3::new(500.0, 0.3, 0.0));
        let mut man = Manoeuvre::default();
        turnaround_car(&mut man, false, -3.0, -3.0, 1.0, &far, 0.2, &m);
        assert_eq!(man, Manoeuvre { reverse: true, turn: 0, handbrake: false });
    }

    #[test]
    fn arc_geometry() {
        let m = open_mesh();
        let car = car_at(Vec3::ZERO);
        // a full circle of radius 5 m fits in the open; of 1.25 x 5 m backing up too
        assert!(arc_clear(&m, &car, false, 0.2, 6.0, false, false));
        assert!(arc_clear(&m, &car, true, 0.2, 6.0, true, false));
        // near the edge (x = 99): turning right (centre +x) leaves the mesh, left does not
        let edge = car_at(Vec3::new(97.0, 0.0, 0.0));
        assert!(!arc_clear(&m, &edge, false, 0.2, -3.0, false, false));
        assert!(arc_clear(&m, &edge, false, 0.2, 3.0, false, false));
    }

    #[test]
    fn grid_from_bounds() {
        let g = ObjectGrid::from_bounds(Vec3::new(-1000.0, 0.0, -500.0), Vec3::new(1000.0, 0.0, 500.0));
        // widened by 20 m along the diagonal (17.9, 8.9), then whole cells: -1040 .. 1040, -520 .. 520
        assert_eq!(g.min, [-1040.0, -520.0]);
        assert_eq!(g.n, [52, 26]);
        assert_eq!(g.cell(-1040.0, -520.0), Some((0, 0)));
        assert_eq!(g.cell(1040.0, 0.0), None);
        assert_eq!(g.key(51, 25), Some(25 * 52 + 51));
        assert_eq!(g.key(52, 0), None);
    }

    fn obj(pos: Vec3, vel: Vec3, kind: u8) -> ObjectInfo {
        ObjectInfo { pos, vel, forward: Vec3::new(0.0, 0.0, -1.0), extent: (2.0, 4.6), kind }
    }

    #[test]
    fn obstacles() {
        let t = AivTuning::default();
        let car = SelCar { pos: Vec3::ZERO, forward: Vec3::new(0.0, 0.0, -1.0), vel: Vec3::new(0.0, 0.0, -20.0), speed: 20.0, half_length: 2.3 };
        let line: Vec<Vec3> = (0..40).map(|k| Vec3::new(0.0, 0.0, -10.0 * k as f32)).collect();
        let objects = vec![
            obj(Vec3::ZERO, car.vel, KIND_RACER),                                  // 0: self
            obj(Vec3::new(1.0, 0.0, -30.0), Vec3::new(0.0, 0.0, -15.0), KIND_PLAYER), // 1: ahead, slower
            obj(Vec3::new(0.0, 0.0, 30.0), Vec3::ZERO, KIND_AMBIENT_CAR),          // 2: behind
            obj(Vec3::new(0.0, 0.0, -50.0), Vec3::new(0.0, 0.0, -40.0), KIND_RACER), // 3: ahead, faster
            obj(Vec3::new(0.0, 0.0, -2.0), Vec3::new(0.0, 0.0, -40.0), KIND_RACER),  // 4: faster, alongside
            obj(Vec3::new(0.0, 8.0, -60.0), Vec3::ZERO, KIND_AMBIENT_CAR),         // 5: 8 m above the line
            obj(Vec3::new(2.0, 0.0, -70.0), Vec3::ZERO, KIND_PED),                 // 6: pedestrian
            obj(Vec3::new(0.0, 0.0, -60.0), Vec3::ZERO, KIND_PROP),                // 7: prop
            obj(Vec3::new(30.0, 0.0, -80.0), Vec3::ZERO, KIND_AMBIENT_CAR),        // 8: 30 m to the side
            obj(Vec3::new(0.0, 0.0, -390.0), Vec3::ZERO, KIND_AMBIENT_CAR),        // 9: beyond the look-ahead
        ];
        let got = select_obstacles_car(0, &objects, &line, &car, 0, &ObjectGrid::unbounded(), &t);
        let pos: Vec<f32> = got.iter().map(|o| o.pos.z).collect();
        // (the walk stops after the point at 50 m: t = 0 + .5 + 1.5 + 3 + 5 + 7.5 > 6 s)
        assert_eq!(pos, vec![-2.0, -30.0, -60.0]);
        // flags: the player is kind 0; faster than the car - 10 -> first leg only
        assert_eq!(got[0].flags, OBST_FIRST_LEG);
        assert_eq!(got[1].flags, OBST_FIRST_LEG | OBST_TYPE0);
        assert_eq!(got[2].flags, 0);
        // direction = -forward (flat, unit); velocity flat
        assert_eq!(got[1].dir, Vec3::new(0.0, 0.0, 1.0));
        assert_eq!(got[1].vel.y, 0.0);
        // the brain's mask: racers ignored
        let got = select_obstacles_car(0, &objects, &line, &car, 1 << KIND_RACER, &ObjectGrid::unbounded(), &t);
        assert_eq!(got.iter().map(|o| o.pos.z).collect::<Vec<_>>(), vec![-30.0, -60.0]);
        // at most 32
        let many: Vec<ObjectInfo> = (0..50).map(|k| obj(Vec3::new((k % 5) as f32, 0.0, -20.0 - (k / 5) as f32), Vec3::ZERO, KIND_AMBIENT_CAR)).collect();
        assert_eq!(select_obstacles_car(usize::MAX, &many, &line, &car, 0, &ObjectGrid::unbounded(), &t).len(), 32);
        // the look-ahead sums each point's distance from the car / 20 m/s: the point at 50 m is the
        // last (an object 10 m past it is found, one 60 m past it is not)
        let far = vec![obj(Vec3::new(0.0, 0.0, -60.0), Vec3::ZERO, KIND_AMBIENT_CAR), obj(Vec3::new(0.0, 0.0, -110.0), Vec3::ZERO, KIND_AMBIENT_CAR)];
        let got = select_obstacles_car(usize::MAX, &far, &line, &car, 0, &ObjectGrid::unbounded(), &t);
        assert_eq!(got.iter().map(|o| o.pos.z).collect::<Vec<_>>(), vec![-60.0]);
    }
}

/// F-59709 (the object grid's cell of a point) and F-74216 (the cell number) against the original's code run offline
/// (tools/people_small_reference.py -> testdata/people_small_ref.txt: `CE`, `KE`). The grid's stored upper bounds are n x 40 + min as
/// the port derives them (the setup that writes them is F-38432, not part of this comparison).
#[cfg(test)]
mod original_tests {
    use super::*;

    /// F-38432 (the grid set-up from the level bounds: widened by 20 m along the diagonal, out to whole 40 m cells; the accumulators
    /// start beyond the bounds, so the points are the bounds) against the original's code run offline (`GR` lines): the cell counts, the
    /// origin and the stored upper bounds (the original computes those as n x 40 + origin, fused).
    /// F-86835 (the racers' polygon cost: 1.0 for the start or goal flags and for road / jump bits, else 5.0) against the original's code run
    /// offline (`RC` lines, 400 flag combinations).
    #[test]
    fn racer_cost_matches_the_original() {
        let hx = |x: &str| u32::from_str_radix(x, 16).unwrap();
        let mut n = 0;
        for line in include_str!("testdata/people_small_ref.txt").lines().filter(|l| l.starts_with("RC ")) {
            let w: Vec<&str> = line.split(' ').collect();
            let cost = mcla_formats::rage::navmesh::racer_cost(hx(w[1]), hx(w[2]));
            assert_eq!(cost(hx(w[3])).to_bits(), hx(w[5]), "{line}");
            n += 1;
        }
        assert_eq!(n, 400);
    }

    #[test]
    fn grid_setup_matches_the_original() {
        let hx = |x: &str| u32::from_str_radix(x, 16).unwrap();
        let mut n = 0;
        for line in include_str!("testdata/people_small_ref.txt").lines().filter(|l| l.starts_with("GR ")) {
            let w: Vec<&str> = line.split(' ').collect();
            let f = |i: usize| f32::from_bits(hx(w[i]));
            let g = ObjectGrid::from_bounds(Vec3::new(f(1), 0.0, f(2)), Vec3::new(f(3), 0.0, f(4)));
            let want_n: [i32; 2] = [w[6].parse().unwrap(), w[7].parse().unwrap()];
            assert_eq!(g.n, want_n, "{line}");
            assert_eq!([g.min[0].to_bits(), g.min[1].to_bits()], [hx(w[8]), hx(w[9])], "{line}");
            assert_eq!([(g.n[0] as f32).mul_add(CELL, g.min[0]).to_bits(), (g.n[1] as f32).mul_add(CELL, g.min[1]).to_bits()], [hx(w[10]), hx(w[11])], "{line}");
            n += 1;
        }
        assert_eq!(n, 301);
    }

    #[test]
    fn grid_cell_and_key_match_the_original() {
        let hx = |x: &str| u32::from_str_radix(x, 16).unwrap();
        let (mut ce, mut ke) = (0, 0);
        for line in include_str!("testdata/people_small_ref.txt").lines() {
            let w: Vec<&str> = line.split(' ').collect();
            if w[0] == "CE" {
                let (nx, nz): (i32, i32) = (w[1].parse().unwrap(), w[2].parse().unwrap());
                let f = |i: usize| f32::from_bits(hx(w[i]));
                let g = ObjectGrid { min: [f(3), f(4)], n: [nx, nz], bounded: true };
                assert_eq!(f(5), nx as f32 * CELL + f(3));
                assert_eq!(f(6), nz as f32 * CELL + f(4));
                let (id, ix, iz): (i32, i32, i32) = (w[10].parse().unwrap(), w[11].parse().unwrap(), w[12].parse().unwrap());
                match g.cell(f(7), f(8)) {
                    None => assert_eq!((id, ix, iz), (-1, 0, 0), "{line}"),
                    Some((x, z)) => {
                        assert_eq!((x, z), (ix, iz), "{line}");
                        assert_eq!(g.key(x, z), Some(id as i64), "{line}");
                    }
                }
                ce += 1;
            } else if w[0] == "KE" {
                let g = ObjectGrid { min: [0.0; 2], n: [w[1].parse().unwrap(), w[2].parse().unwrap()], bounded: true };
                let id: i32 = w[6].parse().unwrap();
                let got = g.key(w[3].parse().unwrap(), w[4].parse().unwrap());
                assert_eq!(got, if id < 0 { None } else { Some(id as i64) }, "{line}");
                ke += 1;
            }
        }
        assert_eq!((ce, ke), (800, 800));
    }

    /// F-53450 (one cell of the obstacle selection) against the original's code run offline (work/racerai/obsel:
    /// synthetic objects on the real mcProp virtuals, the obstacle append replaced by a recorder of its arguments):
    /// 2,000 cases (kinds 0 - 5, ignore masks, the brain's own object, pre-filled taken lists, the aiv's count near 32,
    /// an empty cell, tiny forwards), every handed-on obstacle (pos, vel, dir, extent, both flags) and the taken
    /// count bit for bit.
    #[test]
    fn obstacle_cell_matches_the_original() {
        let fl = |w: &str| f32::from_bits(u32::from_str_radix(w, 16).unwrap());
        let (mut cases, mut obst) = (0, 0);
        for line in include_str!("testdata/obstacle_cell_ref.txt").lines() {
            let (inp, rest) = line.split_once(" |").unwrap();
            let (outs, taken_n) = rest.rsplit_once(" | ").unwrap();
            let t: Vec<&str> = inp.split_whitespace().collect();
            let (n, cnt0, cell_ok, me, ign): (usize, usize, bool, i32, u32) = (t[1].parse().unwrap(), t[2].parse().unwrap(), t[3] == "1", t[4].parse().unwrap(), t[5].parse().unwrap());
            let f: Vec<f32> = t[6..22].iter().map(|w| fl(w)).collect();
            let v3 = |a: usize| Vec3::new(f[a], f[a + 1], f[a + 2]);
            let mut objects = Vec::new();
            let mut k = 22;
            for _ in 0..n {
                let kind: u8 = t[k][1..].parse().unwrap();
                let g: Vec<f32> = t[k + 1..k + 9].iter().map(|w| fl(w)).collect();
                objects.push(ObjectInfo { pos: Vec3::new(g[0], g[1], g[2]), vel: v3(13), forward: -Vec3::new(g[3], g[4], g[5]), extent: (g[6], g[7]), kind });
                k += 9;
            }
            let pre: usize = t[k][1..].parse().unwrap();
            let mut taken: Vec<usize> = t[k + 1..k + 1 + pre].iter().map(|w| w.parse().unwrap()).collect();
            let mut tun = AivTuning::default();
            tun.max_obstacle_height_delta = f[11];
            tun.max_obstacle_flat_delta = f[12];
            let list: Vec<usize> = if cell_ok { (0..n).collect() } else { Vec::new() };
            let mut out = Vec::new();
            select_cell(&list, if me < 0 { usize::MAX } else { me as usize }, &objects, ign, v3(0), v3(3), f[6], f[7], v3(8), &tun, cnt0, &mut taken, &mut out);
            let want: Vec<&str> = outs.split(" o").skip(1).collect();
            assert_eq!(out.len(), want.len(), "case {cases}: obstacles");
            for (o, w) in out.iter().zip(&want) {
                let w: Vec<&str> = w.split_whitespace().collect();
                let got = [o.pos.x, o.pos.y, o.pos.z, o.vel.x, o.vel.y, o.vel.z, o.dir.x, o.dir.y, o.dir.z, o.extent.0, 0.0, o.extent.1];
                for (i, g) in got.iter().enumerate() {
                    assert_eq!(g.to_bits(), u32::from_str_radix(w[i], 16).unwrap(), "case {cases} word {i}");
                }
                assert_eq!((o.flags & OBST_FIRST_LEG != 0) as u32, w[12].parse::<u32>().unwrap(), "case {cases} first leg");
                assert_eq!((o.flags & OBST_TYPE0 != 0) as u32, w[13].parse::<u32>().unwrap(), "case {cases} kind 0");
                obst += 1;
            }
            assert_eq!(taken.len(), taken_n.trim().parse::<usize>().unwrap(), "case {cases}: taken");
            cases += 1;
        }
        assert_eq!((cases, obst), (2000, 986));
    }

    /// F-86521 whole (with the real F-53450, grid helpers and F-41334) run offline by
    /// work/racerai/obsdrv on random cars, control-point rings and objects in a 10 x 8 grid.
    #[test]
    fn obstacle_scan_matches_the_original() {
        let fl = |w: &str| f32::from_bits(u32::from_str_radix(w, 16).unwrap());
        let (mut cases, mut obst) = (0, 0);
        let grid = ObjectGrid { min: [-200.0, -160.0], n: [10, 8], bounded: true };
        for line in include_str!("testdata/obstacle_scan_ref.txt").lines() {
            let (inp, rest) = line.split_once(" |").unwrap();
            let (outs, ign_after) = rest.rsplit_once(" | ").unwrap();
            let t: Vec<&str> = inp.split_whitespace().collect();
            let n: usize = t[1].parse().unwrap();
            let me: i32 = t[2].parse().unwrap();
            let np: usize = t[3].parse().unwrap();
            let ign: u32 = t[6].parse().unwrap();
            let f: Vec<f32> = t[7..24].iter().map(|w| fl(w)).collect();
            let v3 = |a: usize| Vec3::new(f[a], f[a + 1], f[a + 2]);
            let mut k = 24;
            let mut pts = Vec::new();
            for _ in 0..np {
                pts.push(Vec3::new(fl(t[k + 1]), fl(t[k + 2]), fl(t[k + 3])));
                k += 4;
            }
            let mut objects = Vec::new();
            for _ in 0..n {
                let kind: u8 = t[k][1..].parse().unwrap();
                let g: Vec<f32> = t[k + 1..k + 9].iter().map(|w| fl(w)).collect();
                objects.push(ObjectInfo { pos: Vec3::new(g[0], g[1], g[2]), vel: v3(14), forward: -Vec3::new(g[3], g[4], g[5]), extent: (g[6], g[7]), kind });
                k += 9;
            }
            let mut tun = AivTuning::default();
            tun.max_obstacle_height_delta = f[11];
            tun.max_obstacle_flat_delta = f[12];
            tun.obstacle_lookahead_time = f[13];
            let car = SelCar { pos: v3(0), forward: -v3(3), vel: v3(6), speed: f[9], half_length: f[10] };
            let got = select_obstacles_car(if me < 0 { usize::MAX } else { me as usize }, &objects, &pts, &car, ign, &grid, &tun);
            let want: Vec<&str> = outs.split(" o").skip(1).collect();
            assert_eq!(got.len(), want.len(), "case {cases}: obstacles");
            for (o, w) in got.iter().zip(&want) {
                let w: Vec<&str> = w.split_whitespace().collect();
                let g = [o.pos.x, o.pos.y, o.pos.z, o.vel.x, o.vel.y, o.vel.z, o.dir.x, o.dir.y, o.dir.z, o.extent.0, 0.0, o.extent.1];
                for (i, g) in g.iter().enumerate() {
                    assert_eq!(g.to_bits(), u32::from_str_radix(w[i], 16).unwrap(), "case {cases} word {i}");
                }
                assert_eq!((o.flags & OBST_FIRST_LEG != 0) as u32, w[12].parse::<u32>().unwrap(), "case {cases} first leg");
                assert_eq!((o.flags & OBST_TYPE0 != 0) as u32, w[13].parse::<u32>().unwrap(), "case {cases} kind 0");
                obst += 1;
            }
            // brain +408 gains bit 4 for good
            assert_eq!(ign_after.trim().parse::<u32>().unwrap(), ign | 16, "case {cases}: ignore");
            cases += 1;
        }
        assert_eq!((cases, obst), (300, 654));
    }
}
