//! mcBrainRacing: a found nav path handed to the aiv race line, 1:1 with the original
//! (F-47058 and its portal feeders F-77661, F-82239, F-78429; decode in
//! work/racerai/brain.md 2.4, listings work/racerai/scratch/br.txt).
//!
//! The nav result is the original's linked list of 56-byte elements: one portal (type 2) per
//! polygon left, then one point (type 0, the goal point); its count (path +20) is
//! `portals.len() + 1` here.

use crate::aiv::{Aiv, Portal};
use crate::brain::Goal;
use glam::Vec3;
use mcla_formats::rage::navmesh::{NavMesh, Portal56, QuickestPath};

/// Material flag tested by the feeders: vfunc 60 of the mesh (F-43095) returns the
/// polygon's material object, its word +8 is the material flags; `& 2` = ai_road. Span nodes
/// have no material (vfunc 60 returns 0), so their flag is 0.
const FLAG_BIT: u32 = 0x2;

/// fmadds / fmsubs as the recomp evaluates them: fma in double, rounded to single.
fn fma(a: f32, b: f32, c: f32) -> f32 {
    (a as f64).mul_add(b as f64, c as f64) as f32
}

/// F-83219: the polygon's normal-table entry rotated by its chunk matrix (no
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

/// Quad far-edge tables of F-77661: index 2 x (4 a + b) into 0x01010258 / 0x1A4FB3BC.
const QUAD_T1: u32 = 0x0101_0258;
const QUAD_T2: u32 = 0x1A4F_B3BC;

/// F-77661(brain, element, start): the edge of the path's first polygon (`start`: the
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
        // (sraw: the shift's low 6 bits, 32 .. 63 giving 0 for these positive tables)
        let s = (((ia << 2) | ib) << 1) as u32 & 63;
        let t = |w: u32| if s < 32 { (w >> s) as i32 & 3 } else { 0 };
        (t(QUAD_T1), t(QUAD_T2))
    };
    // A degenerate triangle (A and B nearest the same vertex) gives an index outside 0 .. 2: the
    // original reads its stack there - 3 the fetch's fourth vertex, -1 the stored centre, -2 the
    // normal (both written before the read).
    // (approx.: -3 and below or 4 and above need a vertex list with no vertex within 1e4 m of A or B;
    // the port wraps them into 0..3)
    let c = poly.center;
    let nrm = poly_normal(mesh, id);
    let vert = |k: i32| match k {
        -1 => c,
        -2 => nrm,
        _ => v[(k & 3) as usize],
    };
    // orientation against the stored centre (F-58750)
    let (u, w) = (vert(i) - c, vert(j) - c);
    let cross = fma(w.z, u.x, -(u.z * w.x));
    if (cross < 0.0) != start {
        std::mem::swap(&mut i, &mut j);
    }
    Some(Portal { a: vert(i), b: vert(j), normal: nrm, offroad: node_flag(mesh, id), ext: (0.0, 0.0) })
}

fn push_far_edge(aiv: &mut Aiv, mesh: &NavMesh, p: &Portal56, start: bool) -> Option<Portal> {
    let portal = far_edge(mesh, p, start)?;
    aiv.push_portal(&portal);
    Some(portal)
}

/// F-78429(brain, element): the path portal itself, with the clearances beyond its ends as
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

/// F-82239(brain, point): a path with no portal (start and goal in one polygon) on an
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

/// F-47058 with the brain's +216 (`current`, the goal being driven to) and +220 (`next`,
/// the goal the path was planned to). Call once the nav state is 4 and the aiv's goal ring has
/// room (< 8). Returns false when the aiv's portal ring has no room for the path (nothing is
/// done and the nav state stays 4); true when the path was consumed (then the caller sets the
/// nav state to 0, F-71636).
#[allow(clippy::too_many_arguments)]
pub fn path_to_aiv_at(aiv: &mut Aiv, mesh: &NavMesh, path: &QuickestPath, goals: &mut Vec<Goal>, current: usize, next: &mut usize, looping: bool, final_heading: Vec3, final_speed: f32) -> bool {
    let node = if path.portals.is_empty() { goal_node(mesh, path) } else { None };
    path_to_aiv_with(aiv, mesh, path, node, goals, current, next, looping, final_heading, final_speed)
}

/// [`path_to_aiv_at`] with the point element's goal node (+20) given (used only by a path with no portal).
#[allow(clippy::too_many_arguments)]
fn path_to_aiv_with(aiv: &mut Aiv, mesh: &NavMesh, path: &QuickestPath, node: Option<u32>, goals: &mut Vec<Goal>, current: usize, next: &mut usize, looping: bool, final_heading: Vec3, final_speed: f32) -> bool {
    // count = elements of the list (portals + the goal point)
    let count = path.portals.len() as i32 + 1;
    let (mut left, long) = if count + 1 >= 96 { (64, true) } else { (count, false) };
    if left + 1 > Aiv::PORTAL_CAP - aiv.portal_count() {
        return false;
    }
    let mut last: Option<&Portal56> = None;
    for p in &path.portals {
        if left < 0 {
            break;
        }
        if last.is_none() {
            // drop the previous leg's far edge
            aiv.pop_portal();
            // (a leg planned from the car, or only the car's own portal left: base + count == 1)
            if current >= *next || aiv.portal_end() == 1 {
                aiv.pop_portal();
                push_far_edge(aiv, mesh, p, true);
            }
        }
        push_path_portal(aiv, mesh, p);
        last = Some(p);
        left -= 1;
    }
    if let Some(p) = last {
        push_far_edge(aiv, mesh, p, false);
    } else if !aiv.has_path() {
        if let Some(id) = node {
            for p in single_polygon(mesh, id) {
                aiv.push_portal(&p);
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
        // +16 / +28 (F-89800 shifts the array up and fills only +0..+12)
        // The two middles are those of the ends the aiv ring stored (push_portal moves a road
        // portal's ends inward).
        let ((pa, pb), (la, lb)) = aiv.last_two_ends();
        let half = Vec3::splat(0.5);
        let m_prev = (pb + pa) * half;
        let m_last = (lb + la) * half;
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

    /// F-77661, F-82239 and F-78429 run offline (tools/race_line_feeders_reference.py, verify_race_line_feeders.md): every
    /// pushed portal's ends, normal, flag and extensions equal to far_edge, single_polygon and path_portal.
    #[test]
    fn race_line_feeders_match_the_original() {
        let text = include_str!("testdata/race_line_feeders_ref.txt");
        let hex = |w: &str| u32::from_str_radix(&w.replace('_', ""), 16).unwrap();
        let words = |e: &Portal| {
            let mut w: Vec<u32> = [e.a, e.b, e.normal].iter().flat_map(|q| [q.x, q.y, q.z]).map(f32::to_bits).collect();
            w.extend([e.offroad as u32, e.ext.0.to_bits(), e.ext.1.to_bits()]);
            w
        };
        // one polygon (node 0) with the case's vertices, centre, normal (chunk rows the identity) and flags
        let mesh = |nv: u32, verts: [Vec3; 4], center: Vec3, n: Vec3, flags: u32| {
            let mut m = NavMesh::default();
            m.chunk_rows = vec![[Vec3::X, Vec3::Y, Vec3::Z, Vec3::ZERO]];
            m.chunk_first = vec![0];
            m.polys.push(NavPoly { chunk: 0, material: 1, flags, area: 1.0, center, normal: n, nv: nv as u8, verts, first_link: 0, n_links: 0 });
            m.local.push(PolyLocal { verts, n, d: 0.0 });
            m
        };
        let mut counts = [0usize; 3];
        let mut bad = 0;
        for line in text.lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            let arrow = w.iter().position(|x| *x == "->").unwrap();
            let v: Vec<u32> = w[1..arrow].iter().map(|x| hex(x)).collect();
            let o: Vec<u32> = w[arrow + 1..].iter().map(|x| hex(x)).collect();
            let v3 = |i: usize| Vec3::new(f32::from_bits(v[i]), f32::from_bits(v[i + 1]), f32::from_bits(v[i + 2]));
            let vs = |i: usize| [v3(i), v3(i + 3), v3(i + 6), v3(i + 9)];
            let fl = |i: usize| if v[i] != 0 { v[i + 1] } else { 0 };
            let got: Vec<u32> = match w[0] {
                "edge" => {
                    counts[0] += 1;
                    let m = mesh(v[1], vs(8), v3(20), v3(23), fl(26));
                    let p = Portal56 { a: v3(2), b: v3(5), clear_a: 0.0, clear_b: 0.0, left: 0, entered: 0, flags: 0, normal: Vec3::Y };
                    words(&far_edge(&m, &p, v[0] != 0).unwrap())
                }
                "single" => {
                    counts[1] += 1;
                    let m = mesh(v[0], vs(1), Vec3::ZERO, v3(13), fl(16));
                    single_polygon(&m, 0).iter().flat_map(words).collect()
                }
                _ => {
                    counts[2] += 1;
                    let m = mesh(4, [Vec3::ZERO; 4], Vec3::ZERO, v3(8), fl(11));
                    let p = Portal56 { a: v3(0), b: v3(3), clear_a: f32::from_bits(v[6]), clear_b: f32::from_bits(v[7]), left: 0, entered: 0, flags: 0, normal: Vec3::Y };
                    words(&path_portal(&m, &p))
                }
            };
            if got != o {
                if bad < 5 {
                    println!("{line}{}  port {:08x?}", char::from(10), got);
                }
                bad += 1;
            }
        }
        println!("race line feeders: {} edge, {} single, {} portal cases, {bad} differ", counts[0], counts[1], counts[2]);
        assert!(counts[0] >= 8000 && counts[1] >= 2000 && counts[2] >= 2000);
        assert_eq!(bad, 0);
    }

    /// F-47058 run offline with its feeders, the aiv's push / pop and the goal insert as generated (tools/path_to_aiv_reference.py,
    /// verify_path_to_aiv.md): whether the path is consumed, the portal ring (start, count, base, every live portal's ends,
    /// shrunk ends, normal and flag), the goal push's arguments and the brain's goals, flags and +220 equal.
    #[test]
    fn path_to_aiv_matches_the_original() {
        let text = include_str!("testdata/path_to_aiv_ref.txt");
        let hex = |w: &str| u32::from_str_radix(&w.replace('_', ""), 16).unwrap();
        let fnv = |h: u32, v: u32| v.to_le_bytes().iter().fold(h, |h, b| (h ^ *b as u32).wrapping_mul(16_777_619));
        const CL: [f32; 4] = [0.0, 0.4, 1.5, 100.0];
        const TUNE: [u32; 27] = [
            0x14, 0, 1, 0x3f000000, 0x41c80000, 3, 0x3e4ccccd, 0x3dcccccd, 0x40a00000, 0x3f800000, 0x41200000, 0x3dcccccd, 0x3f400000, 0x3dcccccd,
            0x3f800000, 0x3dcac083, 0x3f800000, 0x40c00000, 0x3f800000, 0x3f800000, 0x3f400000, 0x3f800000, 0x3f000000, 0x40a00000, 0x41c80000,
            0x42c80000, 0x40d68f5c,
        ];
        let (mut cases, mut consumed, mut long, mut bad) = (0, 0, 0, 0);
        for line in text.lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            let arrow = w.iter().position(|x| *x == "->").unwrap();
            let v: Vec<u32> = w[1..arrow].iter().map(|x| hex(x)).collect();
            let o: Vec<u32> = w[arrow + 1..].iter().map(|x| hex(x)).collect();
            let f = |i: usize| f32::from_bits(v[i]);
            let v3 = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            let (ng, nd, np) = (v[14] as usize, v[15], v[16] as usize);
            let (g0, p0) = (17, 17 + 9 * ng);
            let t0 = p0 + 84;
            // the aiv: an image of the original's bytes (big endian), read as the recorder's states are
            let mut img = vec![0u8; 31232];
            let mut put = |off: usize, x: u32| img[off..off + 4].copy_from_slice(&x.to_be_bytes());
            let (st, ct, bs) = (v[0], v[1], v[2]);
            for (off, x) in [(22544, (st + ct) % 128), (22548, st), (22552, ct), (22560, bs), (22704, (st + ct) % 128), (22708, st), (22712, ct), (30832, v[3])] {
                put(off, x);
            }
            for i in 0..3 {
                put(25168 + 4 * i, v[4 + i]);
            }
            for (i, x) in TUNE.iter().enumerate() {
                put(25332 + 4 * i, *x);
            }
            let mut aiv = Aiv::new(tune());
            aiv.import_original(&img);
            // the mesh: four polygons (chunk rows the identity)
            let mut m = NavMesh::default();
            m.chunk_rows = vec![[Vec3::X, Vec3::Y, Vec3::Z, Vec3::ZERO]];
            m.chunk_first = vec![0];
            for k in 0..4 {
                let b = p0 + 21 * k;
                let verts = [v3(b + 1), v3(b + 4), v3(b + 7), v3(b + 10)];
                let flags = if v[b + 19] != 0 { v[b + 20] } else { 0 };
                m.polys.push(NavPoly { chunk: 0, material: 1, flags, area: 1.0, center: v3(b + 13), normal: v3(b + 16), nv: v[b] as u8, verts, first_link: 0, n_links: 0 });
                m.local.push(PolyLocal { verts, n: v3(b + 16), d: 0.0 });
            }
            let portals = (0..np)
                .map(|i| {
                    let x = v[t0 + i];
                    let (l, n, ia, ib) = (x & 7, (x >> 3) & 7, (x >> 6) & 3, (x >> 8) & 3);
                    let vt = |k: u32| m.polys[l as usize].verts[k as usize];
                    Portal56 { a: vt(ia), b: vt(ib), clear_a: CL[(x >> 10) as usize & 3], clear_b: CL[(x >> 12) as usize & 3], left: l, entered: n, flags: 0, normal: Vec3::Y }
                })
                .collect();
            let path = QuickestPath { portals, goal: Vec3::ZERO, reached: true };
            let mut goals: Vec<Goal> = (0..ng).map(|i| {
                let b = g0 + 9 * i;
                Goal { pos: v3(b), w: f(b + 3), p1: v3(b + 4), speed: f(b + 7), flags: v[b + 8] }
            }).collect();
            let r30 = v[8] as usize;
            let mut next = r30;
            crate::aiv::GOAL_LOG.with(|l| l.borrow_mut().clear());
            let ok = path_to_aiv_with(&mut aiv, &m, &path, Some(nd), &mut goals, v[7] as usize, &mut next, v[9] != 0, v3(10), f(13));
            let ((ps, pc, pb, _), cells) = aiv.portal_ring_dbg();
            let mut h = 2_166_136_261u32;
            for (q, off) in &cells {
                for e in q {
                    for x in [e.x, e.y, e.z] {
                        h = fnv(h, x.to_bits());
                    }
                }
                h = fnv(h, *off as u32);
            }
            let log = crate::aiv::GOAL_LOG.with(|l| l.borrow().clone());
            let mut got = vec![ok as u32, ps as u32, pc as u32, pb as u32, cells.len() as u32, h, log.len() as u32];
            for (dir, p, gw, q, speed, flags) in &log {
                got.extend([dir.x, dir.y, dir.z, p.x, p.y, p.z, *gw, q.x, q.y, q.z, goals[r30].speed].map(f32::to_bits));
                got.extend([*flags, speed.to_bits()]);
            }
            let gh = goals.iter().flat_map(|g| [g.pos.x, g.pos.y, g.pos.z, g.w, g.p1.x, g.p1.y, g.p1.z, g.speed]).fold(2_166_136_261u32, |h, x| fnv(h, x.to_bits()));
            let fh = goals.iter().fold(2_166_136_261u32, |h, g| fnv(h, g.flags));
            got.extend([goals.len() as u32, goals.len() as u32, next as u32, gh, fh]);
            cases += 1;
            consumed += ok as usize;
            long += (np + 2 >= 96) as usize;
            if got != o {
                if bad < 5 {
                    println!("{}{}  port {:08x?}", &line[..line.len().min(300)], char::from(10), got);
                    println!("  orig {:08x?}", o);
                }
                bad += 1;
            }
        }
        println!("path to aiv: {cases} cases, {consumed} consumed, {long} long, {bad} differ");
        assert!(cases >= 3000);
        assert_eq!(bad, 0);
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
