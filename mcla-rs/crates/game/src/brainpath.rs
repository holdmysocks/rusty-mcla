//! mcBrainRacing: a found nav path handed to the aiv race line, 1:1 with the original
//! (sub_822BE000 and its portal feeders sub_822BBEE8, sub_822BC360, sub_822BC508; decode in
//! work/racerai/brain.md 2.4, listings work/racerai/scratch/br.txt).
//!
//! The nav result is the original's linked list of 56-byte elements: one portal (type 2) per
//! polygon left, then one point (type 0, the goal point); its count (path +20) is
//! `portals.len() + 1` here.

use crate::aiv::{Aiv, Portal};
use crate::brain::Goal;
use glam::Vec3;
use mcla_formats::rage::navmesh::{NavMesh, Portal56, QuickestPath};

/// Material flag tested by the feeders: vfunc 60 of the mesh (sub_82764828) returns the
/// polygon's material object, its word +8 is the material flags; `& 2` = ai_road. Span nodes
/// have no material (vfunc 60 returns 0), so their flag is 0.
const FLAG_BIT: u32 = 0x2;

/// fmadds / fmsubs as the recomp evaluates them: fma in double, rounded to single.
fn fma(a: f32, b: f32, c: f32) -> f32 {
    (a as f64).mul_add(b as f64, c as f64) as f32
}

/// sub_8254F478: the polygon's normal-table entry rotated by its chunk matrix (no
/// normalisation).
fn poly_normal(mesh: &NavMesh, id: u32) -> Vec3 {
    let p = &mesh.polys[id as usize];
    let (Some(l), Some(r)) = (mesh.local.get(id as usize), mesh.chunk_rows.get(p.chunk as usize)) else {
        return p.normal;
    };
    r[0] * l.n.x + r[1] * l.n.y + r[2] * l.n.z
}

/// vfunc 60 +8 & 2 for a node id of the port (polygons; span nodes = `polys.len() + record`).
fn node_flag(mesh: &NavMesh, id: u32) -> bool {
    mesh.polys.get(id as usize).map_or(false, |p| p.flags & FLAG_BIT != 0)
}

/// Quad far-edge tables of sub_822BBEE8: index 2 x (4 a + b) into 0x01010258 / 0x1A4FB3BC.
const QUAD_T1: u32 = 0x0101_0258;
const QUAD_T2: u32 = 0x1A4F_B3BC;

/// sub_822BBEE8(brain, element, start): the edge of the path's first polygon (`start`: the
/// polygon left by the portal) or last polygon (the polygon entered) opposite the portal,
/// pushed so the corridor covers that whole polygon.
fn far_edge(mesh: &NavMesh, p: &Portal56, start: bool) -> Option<Portal> {
    let id = if start { p.left } else { p.entered };
    // (approx.: a span node here makes the original read a chunk / polygon from the span id's
    // bits (chunk = span >> 2, polygon = (span & 3) << 10 | record) - nothing meaningful;
    // the port pushes nothing, as the original does when that chunk test fails)
    let poly = mesh.polys.get(id as usize)?;
    let nv = poly.nv as usize;
    let v = &poly.verts;
    // vertices nearest to A and to B in plan (strict <, from 1e8, in vertex order)
    let (mut ia, mut ib) = (-1i32, -1i32);
    let (mut da, mut db) = (1.0e8f32, 1.0e8f32);
    for (k, q) in v.iter().enumerate().take(nv) {
        let (dx, dz) = (p.a.x - q.x, p.a.z - q.z);
        let d = fma(dz, dz, dx * dx);
        if d < da {
            ia = k as i32;
            da = d;
        }
        let (dx, dz) = (p.b.x - q.x, p.b.z - q.z);
        let d = fma(dz, dz, dx * dx);
        if d < db {
            ib = k as i32;
            db = d;
        }
    }
    let (mut i, mut j) = if nv == 3 {
        // triangle: the vertex nearest A and the third one
        (ia, 3 - ib - ia)
    } else {
        let s = (((ia << 2) | ib) << 1) as u32 & 31;
        ((QUAD_T1 >> s) as i32 & 3, (QUAD_T2 >> s) as i32 & 3)
    };
    // (approx.: a degenerate match (A and B nearest the same vertex) gives the original an
    // index outside the vertex list; the port wraps it into 0..3)
    let vert = |k: i32| v[(k & 3) as usize];
    // orientation against the stored centre (sub_8254F388)
    let c = poly.center;
    let (u, w) = (vert(i) - c, vert(j) - c);
    let cross = fma(w.z, u.x, -(u.z * w.x));
    if (cross < 0.0) != start {
        std::mem::swap(&mut i, &mut j);
    }
    Some(Portal { a: vert(i), b: vert(j), normal: poly_normal(mesh, id), offroad: node_flag(mesh, id), ext: (0.0, 0.0) })
}

fn push_far_edge(aiv: &mut Aiv, mesh: &NavMesh, p: &Portal56, start: bool) -> Option<Portal> {
    let portal = far_edge(mesh, p, start)?;
    aiv.push_portal(&portal);
    Some(portal)
}

/// sub_822BC508(brain, element): the path portal itself, with the clearances beyond its ends as
/// the extensions and the normal / flag of the node entered.
fn path_portal(mesh: &NavMesh, p: &Portal56) -> Portal {
    let normal = if (p.entered as usize) < mesh.polys.len() {
        poly_normal(mesh, p.entered)
    } else {
        // (approx.: for a span node entered the original looks the normal up in a chunk taken
        // from the span id's bits (see push_far_edge), default (0, 1, 0) at 0x827D8480 when that
        // chunk is not loaded; the port uses the portal's stored normal)
        p.normal
    };
    Portal { a: p.a, b: p.b, normal, offroad: node_flag(mesh, p.entered), ext: (p.clear_a, p.clear_b) }
}

fn push_path_portal(aiv: &mut Aiv, mesh: &NavMesh, p: &Portal56) -> Portal {
    let portal = path_portal(mesh, p);
    aiv.push_portal(&portal);
    portal
}

/// sub_822BC360(brain, point): a path with no portal (start and goal in one polygon) on an
/// empty corridor: two edges of that polygon, v0-v1 and (v0 for a triangle, else v3)-v2.
fn single_polygon(mesh: &NavMesh, id: u32) -> Vec<Portal> {
    let Some(poly) = mesh.polys.get(id as usize) else { return Vec::new() };
    let (n, f) = (poly_normal(mesh, id), node_flag(mesh, id));
    let v = &poly.verts;
    [(v[0], v[1]), (if poly.nv == 3 { v[0] } else { v[3] }, v[2])].into_iter().map(|(a, b)| Portal { a, b, normal: n, offroad: f, ext: (0.0, 0.0) }).collect()
}

/// The goal node of the path's point element (+20).
/// (approx.: `QuickestPath` does not carry it; the port takes the polygon under the goal point
/// within 1 m. navmesh.rs should add `goal_node: u32` to `QuickestPath` - the start polygon
/// found to be a goal, the goal polygon popped, or the closest node.)
fn goal_node(mesh: &NavMesh, path: &QuickestPath) -> Option<u32> {
    mesh.find(path.goal, u32::MAX, 1.0)
}

/// sub_822BE000 with the brain's +216 (`current`, the goal being driven to) and +220 (`next`,
/// the goal the path was planned to). Call once the nav state is 4 and the aiv's goal ring has
/// room (< 8). Returns false when the aiv's portal ring has no room for the path (nothing is
/// done and the nav state stays 4); true when the path was consumed (then the caller sets the
/// nav state to 0, sub_826FD488).
#[allow(clippy::too_many_arguments)]
pub fn path_to_aiv_at(aiv: &mut Aiv, mesh: &NavMesh, path: &QuickestPath, goals: &mut Vec<Goal>, current: usize, next: &mut usize, looping: bool, final_heading: Vec3, final_speed: f32) -> bool {
    // count = elements of the list (portals + the goal point)
    let count = path.portals.len() as i32 + 1;
    let (mut left, long) = if count + 1 >= 96 { (64, true) } else { (count, false) };
    if left + 1 > Aiv::PORTAL_CAP - aiv.portal_count() {
        return false;
    }
    let mut pushed: Vec<Portal> = Vec::new();
    let mut last: Option<&Portal56> = None;
    for p in &path.portals {
        if left < 0 {
            break;
        }
        if last.is_none() {
            // drop the previous leg's far edge
            aiv.pop_portal();
            // (approx.: the original tests aiv+22560 + aiv+22552 == 1 (portal ring base + count);
            // aiv.rs exposes no base - it should add `pub fn portal_end(&self) -> i32` - so the
            // port takes the base as 0)
            if current >= *next || aiv.portal_count() == 1 {
                aiv.pop_portal();
                pushed.extend(push_far_edge(aiv, mesh, p, true));
            }
        }
        pushed.push(push_path_portal(aiv, mesh, p));
        last = Some(p);
        left -= 1;
    }
    if let Some(p) = last {
        pushed.extend(push_far_edge(aiv, mesh, p, false));
    } else if !aiv.has_path() {
        if let Some(id) = goal_node(mesh, path) {
            for p in single_polygon(mesh, id) {
                aiv.push_portal(&p);
                pushed.push(p);
            }
        }
    }

    // ---- the goal to the aiv ----
    let r30 = *next;
    let tmpl = goals.get(r30).copied().unwrap_or(Goal { pos: Vec3::ZERO, w: 0.0, p1: Vec3::ZERO, speed: -1.0, flags: 0 });
    let (dir, speed);
    if long {
        // an extra goal between the middles of the last two portals (portal 64 and the far edge
        // of the polygon it enters), radius 100 (0x82007928), max speed -1 (0x82003770); it
        // keeps the following goal's lap / checkpoint bits with bits 0 and 1 cleared, and its
        // +16 / +28 (sub_822BB278 shifts the array up and fills only +0..+12)
        // (approx.: the original reads the two midpoints from the aiv ring's stored ends; the
        // port uses the ends it passed - the same middles up to rounding, push_portal moves both
        // ends of a road portal inward by the same amount)
        let n = pushed.len();
        let (pl, ps) = if n >= 2 { (pushed[n - 1], pushed[n - 2]) } else { (tmpl_portal(&tmpl), tmpl_portal(&tmpl)) };
        let half = Vec3::splat(0.5);
        let m_prev = (ps.a + ps.b) * half;
        let m_last = (pl.a + pl.b) * half;
        dir = m_last - m_prev;
        let pos = (m_last + m_prev) * half;
        let g = Goal { pos, w: 100.0, p1: tmpl.p1, speed: tmpl.speed, flags: tmpl.flags & !3 };
        let at = r30.min(goals.len());
        goals.insert(at, g);
        speed = -1.0;
        *next = r30 + 1;
    } else if (r30 as i64) < goals.len() as i64 - 1 {
        let prev = r30.saturating_sub(1);
        dir = goals[r30 + 1].centre() - goals[prev].centre();
        speed = goals[r30].speed;
        *next = r30 + 1;
    } else if looping {
        let prev = r30.saturating_sub(1);
        dir = goals.first().map_or(Vec3::ZERO, |g| g.centre()) - goals.get(prev).map_or(Vec3::ZERO, |g| g.centre());
        speed = tmpl.speed;
        *next = 0;
    } else {
        dir = final_heading;
        speed = final_speed;
        *next = r30 + 1;
    }
    // (approx.: with +220 past the goal array the original reads beyond it; the port pushes no goal)
    if let Some(g) = goals.get(r30) {
        aiv.push_goal(dir, g.pos, g.w, g.p1, speed, g.flags);
    }
    true
}

fn tmpl_portal(g: &Goal) -> Portal {
    Portal { a: g.pos, b: g.pos, normal: Vec3::Y, offroad: false, ext: (0.0, 0.0) }
}

/// [`path_to_aiv_at`] without the brain's +216.
/// (approx.: +216 >= +220 - the leg planned from the car - is taken as "the aiv has no path";
/// callers should use `path_to_aiv_at` with +216 and keep the nav state at 4 when it returns
/// false)
#[allow(clippy::too_many_arguments)]
pub fn path_to_aiv(aiv: &mut Aiv, mesh: &NavMesh, path: &QuickestPath, goals: &mut Vec<Goal>, next: &mut usize, looping: bool, final_heading: Vec3, final_speed: f32) -> bool {
    let current = if aiv.has_path() { 0 } else { *next };
    path_to_aiv_at(aiv, mesh, path, goals, current, next, looping, final_heading, final_speed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mcla_formats::rage::aitune::AiTune;
    use mcla_formats::rage::navmesh::{NavPoly, PolyLocal};
    use std::sync::Arc;

    fn tune() -> Arc<AiTune> {
        let row = |v: f32| vec![v; 128];
        Arc::new(AiTune {
            max_speed: 60.0,
            min_curvature: 0.005,
            max_curvature: 0.2,
            max_total_accel: 30.0,
            max_brake_decel: 20.0,
            idle_speed: [0.0; 7],
            accel: row(6.0),
            decel: (0..7).map(|_| row(3.0)).collect(),
            speed_high_curv: vec![30.0; 64],
            speed_low_curv: vec![60.0; 64],
            steer_scalar: row(4.0),
            drag: [0.0; 3],
            accel_rows: (0..8).map(|_| row(6.0)).collect(),
        })
    }

    /// A strip of `n` 10 m road squares along +x (vertices counter-clockwise from (x0, 0)).
    fn strip(n: usize) -> NavMesh {
        let mut m = NavMesh::default();
        m.chunk_rows = vec![[Vec3::X, Vec3::Y, Vec3::Z, Vec3::ZERO]];
        m.chunk_first = vec![0];
        for k in 0..n {
            let x = 10.0 * k as f32;
            let verts = [Vec3::new(x, 0.0, 0.0), Vec3::new(x + 10.0, 0.0, 0.0), Vec3::new(x + 10.0, 0.0, 10.0), Vec3::new(x, 0.0, 10.0)];
            m.polys.push(NavPoly { chunk: 0, material: 1, flags: 0x82, area: 100.0, center: Vec3::new(x + 5.0, 0.0, 5.0), normal: Vec3::Y, nv: 4, verts, first_link: 0, n_links: 0 });
            m.local.push(PolyLocal { verts, n: Vec3::Y, d: 0.0 });
        }
        m
    }

    /// The path from square 0 to square `n - 1`: one portal per square left.
    fn path(m: &NavMesh, n: usize) -> QuickestPath {
        let portals = (0..n - 1)
            .map(|k| {
                let x = 10.0 * (k + 1) as f32;
                Portal56 { a: Vec3::new(x, 0.0, 0.0), b: Vec3::new(x, 0.0, 10.0), clear_a: 0.4, clear_b: 0.6, left: k as u32, entered: k as u32 + 1, flags: m.polys[k + 1].flags, normal: Vec3::Y }
            })
            .collect();
        QuickestPath { portals, goal: Vec3::new(10.0 * n as f32 - 5.0, 0.0, 5.0), reached: true }
    }

    fn goal(pos: Vec3, flags: u32) -> Goal {
        Goal { pos, w: 8.0, p1: pos, speed: 20.0, flags }
    }

    #[test]
    fn far_edges_of_two_squares() {
        let m = strip(2);
        let p = path(&m, 2);
        let s = far_edge(&m, &p.portals[0], true).unwrap();
        assert_eq!((s.a, s.b), (Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 10.0)));
        let e = far_edge(&m, &p.portals[0], false).unwrap();
        assert_eq!((e.a, e.b), (Vec3::new(20.0, 0.0, 0.0), Vec3::new(20.0, 0.0, 10.0)));
        assert!(s.offroad && e.offroad && s.ext == (0.0, 0.0));
        let q = path_portal(&m, &p.portals[0]);
        assert_eq!((q.a, q.b, q.ext, q.offroad), (p.portals[0].a, p.portals[0].b, (0.4, 0.6), true));
        // a triangle: the vertex nearest A and the third one, oriented the other way round
        let mut t = strip(1);
        t.polys[0].nv = 3;
        t.polys[0].verts = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 10.0), Vec3::ZERO];
        t.polys[0].center = Vec3::new(20.0 / 3.0, 0.0, 10.0 / 3.0);
        let pt = Portal56 { a: Vec3::new(10.0, 0.0, 0.0), b: Vec3::new(10.0, 0.0, 10.0), clear_a: 0.0, clear_b: 0.0, left: 0, entered: 9, flags: 0, normal: Vec3::Y };
        let e = far_edge(&t, &pt, true).unwrap();
        // ia = 1, ib = 2 -> (1, 3 - 2 - 1 = 0); cross < 0 with start -> kept
        assert_eq!((e.a, e.b), (Vec3::new(10.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)));
        // single-polygon path: v0-v1 and v3-v2
        let sp = single_polygon(&m, 1);
        assert_eq!((sp[0].a, sp[0].b, sp[1].a, sp[1].b), (m.polys[1].verts[0], m.polys[1].verts[1], m.polys[1].verts[3], m.polys[1].verts[2]));
    }

    #[test]
    fn two_squares_from_the_car_then_a_second_leg() {
        let m = strip(3);
        let mut aiv = Aiv::new(tune());
        let mut goals = vec![goal(Vec3::new(15.0, 0.0, 5.0), 0x2), goal(Vec3::new(25.0, 0.0, 5.0), 0x2 | (1 << 6))];
        let mut next = 0;
        assert!(path_to_aiv_at(&mut aiv, &m, &path(&m, 2), &mut goals, 0, &mut next, false, Vec3::Z, -1.0));
        // start far edge + portal + goal far edge
        assert_eq!((aiv.portal_count(), aiv.goal_count(), next, goals.len()), (3, 1, 1, 2));
        // the next leg (square 1 -> 2) drops the far edge and adds the portal and a new far edge
        let p2 = QuickestPath { portals: vec![path(&m, 3).portals[1]], goal: Vec3::new(25.0, 0.0, 5.0), reached: true };
        assert!(path_to_aiv_at(&mut aiv, &m, &p2, &mut goals, 0, &mut next, false, Vec3::Z, -1.0));
        assert_eq!((aiv.portal_count(), aiv.goal_count(), next), (4, 2, 2));
    }

    #[test]
    fn last_goal_looping_and_final() {
        let m = strip(2);
        for (looping, want) in [(true, 0), (false, 1)] {
            let mut aiv = Aiv::new(tune());
            let mut goals = vec![goal(Vec3::new(15.0, 0.0, 5.0), 2)];
            let mut next = 0;
            assert!(path_to_aiv_at(&mut aiv, &m, &path(&m, 2), &mut goals, 0, &mut next, looping, Vec3::Z, 30.0));
            assert_eq!((next, aiv.goal_count()), (want, 1));
        }
    }

    #[test]
    fn long_path_rule() {
        // count = portals + 1; the rule applies from count + 1 >= 96, i.e. 94 portals
        for (n_portals, long) in [(93usize, false), (94, true), (120, true)] {
            let m = strip(n_portals + 1);
            let mut aiv = Aiv::new(tune());
            let flags = 0x2 | (3 << 2) | (5 << 6);
            let far = Vec3::new(10.0 * (n_portals + 1) as f32 - 5.0, 0.0, 5.0);
            let mut goals = vec![goal(far, flags)];
            let mut next = 0;
            assert!(path_to_aiv_at(&mut aiv, &m, &path(&m, n_portals + 1), &mut goals, 0, &mut next, false, Vec3::X, -1.0));
            if long {
                // start edge + portals 0..=64 + far edge of the square portal 64 enters
                assert_eq!(aiv.portal_count(), 67);
                assert_eq!((goals.len(), next, aiv.goal_count()), (2, 1, 1));
                let g = goals[0];
                // middle between portal 64 (x = 650) and the far edge of square 65 (x = 660)
                assert_eq!((g.pos, g.w, g.flags, g.speed), (Vec3::new(655.0, 0.0, 5.0), 100.0, flags & !3, 20.0));
                assert_eq!(goals[1].pos, far);
                // no room for another long path: 65 > 128 - 67
                let before = (aiv.portal_count(), next);
                assert!(!path_to_aiv_at(&mut aiv, &m, &path(&m, n_portals + 1), &mut goals, 0, &mut next, false, Vec3::X, -1.0));
                assert_eq!((aiv.portal_count(), next), before);
            } else {
                assert_eq!(aiv.portal_count(), n_portals as i32 + 2);
                assert_eq!((goals.len(), next), (1, 1));
            }
        }
    }
}
