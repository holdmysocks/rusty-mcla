//! Finding a way where there is no road: the stretches of a race route that leave the
//! lane network (alleys, car parks, plazas, the shortcuts the race scripts list as a few
//! widely spaced nodes).
//!
//! A grid search over the collision mesh: a cell can be stepped to when there is ground
//! at about the same height and nothing stands between the two cells at bumper and at
//! bonnet height, a car's width wide. Cells are only examined when the search reaches
//! them, so a typical stretch costs a few thousand ray casts.

use crate::physics::collision::WorldCollider;
use glam::Vec3;
use std::collections::{BinaryHeap, HashMap};

const CELL: f32 = 2.5;

#[derive(PartialEq)]
struct Open(f32, (i32, i32));
impl Eq for Open {}
impl Ord for Open {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        o.0.partial_cmp(&self.0).unwrap_or(std::cmp::Ordering::Equal)
    }
}
impl PartialOrd for Open {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

fn ground(col: &WorldCollider, x: f32, y: f32, z: f32) -> Option<f32> {
    col.raycast(Vec3::new(x, y + 1.7, z), -Vec3::Y, 3.7).map(|h| h.point.y)
}

/// Nothing in the way between two points a step apart, for a car's width.
fn clear(col: &WorldCollider, a: Vec3, b: Vec3) -> bool {
    let d = b - a;
    let len = d.length();
    if len < 1e-3 {
        return true;
    }
    let dir = d / len;
    let side = Vec3::new(-dir.z, 0.0, dir.x).normalize_or_zero() * 1.05;
    for (off, h) in [(Vec3::ZERO, 0.45), (Vec3::ZERO, 1.15), (side, 0.6), (-side, 0.6)] {
        if col.raycast(a + off + Vec3::Y * h, dir, len + 0.6).is_some() {
            return false;
        }
    }
    true
}

/// One leg: from `a` to within reach of `b`. Returns the points in between (without `a`)
/// and whether the goal was reached; if not, the line leads as close to it as the search got.
fn leg(col: &WorldCollider, solid: &[Vec3], a: Vec3, b: Vec3) -> Option<(Vec<Vec3>, bool)> {
    let ya = ground(col, a.x, a.y, a.z).or_else(|| ground(col, a.x, a.y + 2.0, a.z))?;
    let cell = |p: Vec3| ((p.x / CELL).floor() as i32, (p.z / CELL).floor() as i32);
    let centre = |c: (i32, i32)| ((c.0 as f32 + 0.5) * CELL, (c.1 as f32 + 0.5) * CELL);
    let start = cell(a);
    let goal = cell(b);
    // stay within a corridor around the straight line
    let margin = (45.0 / CELL) as i32;
    let (x0, x1) = (start.0.min(goal.0) - margin, start.0.max(goal.0) + margin);
    let (z0, z1) = (start.1.min(goal.1) - margin, start.1.max(goal.1) + margin);
    let h = |c: (i32, i32)| {
        let (dx, dz) = ((c.0 - goal.0) as f32, (c.1 - goal.1) as f32);
        (dx * dx + dz * dz).sqrt() * CELL
    };
    // per cell: height, cost so far, where it was reached from
    let mut seen: HashMap<(i32, i32), (f32, f32, (i32, i32))> = HashMap::new();
    let mut open = BinaryHeap::new();
    seen.insert(start, (ya, 0.0, start));
    open.push(Open(h(start), start));
    let mut expanded = 0;
    let mut reached: Option<(i32, i32)> = None;
    let mut best = (h(start), start);
    while let Some(Open(_, c)) = open.pop() {
        let (yc, gc, _) = seen[&c];
        if h(c) < best.0 {
            best = (h(c), c);
        }
        if (c.0 - goal.0).abs() <= 1 && (c.1 - goal.1).abs() <= 1 && (yc - b.y).abs() < 4.0 {
            reached = Some(c);
            break;
        }
        expanded += 1;
        if expanded > 5000 {
            break;
        }
        let (cx, cz) = centre(c);
        let pc = Vec3::new(cx, yc, cz);
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
            let n = (c.0 + dx, c.1 + dz);
            if n.0 < x0 || n.0 > x1 || n.1 < z0 || n.1 > z1 {
                continue;
            }
            let step = if dx != 0 && dz != 0 { CELL * 1.4142 } else { CELL };
            let g = gc + step;
            if seen.get(&n).map_or(false, |s| s.1 <= g) {
                continue;
            }
            let (nx, nz) = centre(n);
            let Some(yn) = ground(col, nx, yc, nz) else { continue };
            // ramps yes, kerbs yes, walls and drops no
            if (yn - yc).abs() > step * 0.42 {
                continue;
            }
            let pn = Vec3::new(nx, yn, nz);
            if solid.iter().any(|s| (s.x - nx).abs() < 1.7 && (s.z - nz).abs() < 1.7 && (s.y - yn).abs() < 4.0) {
                continue;
            }
            if !clear(col, pc, pn) {
                continue;
            }
            seen.insert(n, (yn, g, c));
            open.push(Open(g + h(n), n));
        }
    }
    let complete = reached.is_some();
    if !complete && best.0 > h(start) - 12.0 {
        return None;
    }
    let mut c = reached.unwrap_or(best.1);
    let mut out = Vec::new();
    while c != start {
        let (y, _, from) = seen[&c];
        let (x, z) = centre(c);
        out.push(Vec3::new(x, y + 0.2, z));
        c = from;
    }
    out.reverse();
    Some((out, complete))
}

/// The grid search's line through the given points (first and last are kept): the planner
/// before the navigation mesh was read, now the fallback.
fn plan_grid(col: &WorldCollider, solid: &[Vec3], pts: &[Vec3]) -> Option<(Vec<Vec3>, bool)> {
    if pts.len() < 2 {
        return None;
    }
    let mut line = vec![pts[0]];
    let mut complete = true;
    for w in pts.windows(2) {
        let from = *line.last().unwrap();
        let (mut part, done) = leg(col, solid, from, w[1])?;
        line.append(&mut part);
        if !done {
            complete = false;
            break;
        }
    }
    // round the grid's corners off (twice: a car can follow that)
    for _ in 0..2 {
        let src = line.clone();
        for i in 1..line.len().saturating_sub(1) {
            let m = (src[i - 1] + src[i] * 2.0 + src[i + 1]) * 0.25;
            // only where the rounded point is still free
            if clear(col, src[i - 1], m) && clear(col, m, src[i + 1]) {
                line[i] = m;
            }
        }
    }
    if complete {
        line.push(*pts.last().unwrap());
    }
    Some((line, complete))
}

// ---------------------------------------------------------------------------------
// Over the original's AI navigation mesh (`city.xsm`, formats/rage/navmesh.rs): the way
// from one place to the next is searched over its road and shortcut polygons, which is what
// the original's drivers use off the lane graph. The grid search above remains as the
// fallback (no file, no way over the mesh, or the mesh's way is blocked in the port's
// collision) and behind MCLA_OFFROAD_OLD=1. Notes: work/map/unknown_files.md, "AI shortcuts
// on the navigation mesh".

use mcla_formats::rage::navmesh::{NavMesh, AI_DRIVE};

/// spacing of the line's points (m)
const STEP: f32 = 3.0;
/// how far the line keeps from the edge of what is drivable (half a car and a little)
const HALF: f32 = 1.3;
/// how far a route node may lie from the mesh
const SNAP: f32 = 12.0;

pub fn mesh() -> Option<&'static NavMesh> {
    static MESH: std::sync::OnceLock<Option<NavMesh>> = std::sync::OnceLock::new();
    MESH.get_or_init(|| {
        // (the data directory as main.rs takes it: --data, else the default)
        let args: Vec<String> = std::env::args().collect();
        let dir = args.iter().position(|a| a == "--data").and_then(|i| args.get(i + 1).cloned()).or_else(|| std::env::var("MCLA_DATA_DIR").ok()).unwrap_or_else(|| "D:/MCLA/extracted/cache".to_string());
        let m = crate::assets::load_resource_file(&std::path::Path::new(&dir).join("resources/city/sc/0x69d17d57.r26")).ok().and_then(|r| NavMesh::parse(&r));
        match m.as_ref() {
            Some(m) => log::info!("off-road: navigation mesh, {} polygons ({} for cars)", m.polys.len(), m.polys.iter().filter(|p| p.drivable()).count()),
            None => log::warn!("off-road: no navigation mesh; the grid search is used"),
        }
        m
    })
    .as_ref()
}

/// The line over the mesh from `a` to `b`: points every 3 m at the mesh's height + 0.2,
/// corners rounded, kept a car's half width inside the drivable polygons and clear of
/// `solid`. Without `a`; `b` is its last point. `None`: no way, or an absurd detour.
pub fn mesh_line(mesh: &NavMesh, solid: &[Vec3], a: Vec3, b: Vec3) -> Option<Vec<Vec3>> {
    let path = mesh.drive_path(a, b, SNAP, HALF)?;
    if path.length > a.distance(b) * 4.0 + 150.0 || path.points.len() < 2 {
        return None;
    }
    let flags = if path.strict { AI_DRIVE } else { u32::MAX };
    let on = |p: Vec3| mesh.find(p, flags, 3.0).is_some();
    let near_solid = |p: Vec3, r: f32| solid.iter().find(|s| (s.x - p.x).abs() < r && (s.z - p.z).abs() < r && (s.y - p.y).abs() < 4.0).copied();
    // evenly spaced along a line
    let spaced = |pts: &[Vec3]| -> Vec<Vec3> {
        let total: f32 = pts.windows(2).map(|w| w[0].distance(w[1])).sum();
        let mut line: Vec<Vec3> = vec![pts[0]];
        let n = (total / STEP).ceil().max(1.0) as usize;
        let (mut seg, mut done) = (0usize, 0.0f32);
        for k in 1..n {
            let want = total * k as f32 / n as f32;
            while seg + 2 < pts.len() && done + pts[seg].distance(pts[seg + 1]) < want {
                done += pts[seg].distance(pts[seg + 1]);
                seg += 1;
            }
            let (p, q) = (pts[seg], pts[seg + 1]);
            line.push(p.lerp(q, ((want - done) / p.distance(q).max(1e-3)).clamp(0.0, 1.0)));
        }
        line.push(*pts.last().unwrap());
        line
    };
    let mut line = spaced(&path.points);
    // (the pulled-tight line's own heights are those of its corners only: each point takes the
    // height of the polygon of the way it lies on - where roads cross over one another that is
    // the right floor - or else of the mesh under it, looked for at the height of the point before)
    let level = |line: &mut Vec<Vec3>| {
        let mut y = line[0].y;
        for p in line.iter_mut() {
            let own = path.polys.iter().map(|i| &mesh.polys[*i as usize]).filter(|q| q.nv >= 3 && q.contains(p.x, p.z)).map(|q| q.height(p.x, p.z)).min_by(|a, b| (a - y).abs().total_cmp(&(b - y).abs()));
            y = own.filter(|h| (h - y).abs() < 6.0).or_else(|| mesh.height_at(Vec3::new(p.x, y, p.z), flags, 3.0)).unwrap_or(y);
            p.y = y;
        }
    };
    level(&mut line);
    // (a few polygons are not convex, and there the pulled-tight line can leave the mesh:
    // then the plain line through the middles of the edges)
    if !line.iter().all(|p| mesh.nearest(*p, 0.3, 3.0, &|q| !path.strict || q.drivable()).is_some()) {
        line = spaced(&path.mids);
        level(&mut line);
    }
    // round what stands in the way (trees, posts: not part of the mesh)
    for i in 1..line.len().saturating_sub(1) {
        if let Some(s) = near_solid(line[i], 1.9) {
            let along = line[i + 1] - line[i - 1];
            let side = Vec3::new(-along.z, 0.0, along.x).normalize_or(Vec3::X);
            let away = if (line[i] - s).dot(side) >= 0.0 { side } else { -side };
            let q = Vec3::new(s.x, line[i].y, s.z) + away * 2.2;
            if on(q) {
                line[i] = q;
            }
        }
    }
    // round the corners off, where the rounded point still has a car's width of mesh
    for _ in 0..3 {
        let src = line.clone();
        for i in 1..line.len().saturating_sub(1) {
            let m = (src[i - 1] + src[i] * 2.0 + src[i + 1]) * 0.25;
            let along = src[i + 1] - src[i - 1];
            let side = Vec3::new(-along.z, 0.0, along.x).normalize_or(Vec3::X) * HALF;
            if on(m) && on(m + side) && on(m - side) && near_solid(m, 1.7).is_none() {
                line[i] = m;
            }
        }
    }
    level(&mut line);
    let mut out: Vec<Vec3> = line[1..].iter().map(|p| *p + Vec3::Y * 0.2).collect();
    // (the route's own point last, as the callers look for it)
    if out.last().map_or(true, |p| p.distance(b) > 0.05) {
        if out.last().map_or(false, |p| Vec3::new(p.x - b.x, 0.0, p.z - b.z).length() < 1.0) {
            out.pop();
        }
        out.push(b);
    }
    Some(out)
}

/// The mesh's line through the points, checked against the port's collision where that is
/// loaded: `Err(n)` when `n` of its steps are blocked there.
fn plan_mesh(mesh: &NavMesh, col: &WorldCollider, solid: &[Vec3], pts: &[Vec3]) -> Option<Result<Vec<Vec3>, usize>> {
    let mut line = vec![pts[0]];
    for w in pts.windows(2) {
        line.append(&mut mesh_line(mesh, solid, w[0], w[1])?);
    }
    // onto the port's ground where there is some at about that height
    let last = line.len() - 1;
    let floor_at = |p: Vec3| ground(col, p.x, p.y, p.z).filter(|g| (g - p.y).abs() < 1.5).map(|g| Vec3::new(p.x, g, p.z));
    let floor: Vec<Option<Vec3>> = line.iter().map(|p| floor_at(*p)).collect();
    let open = |p: Option<Vec3>, q: Option<Vec3>| !matches!((p, q), (Some(p), Some(q)) if !clear(col, p, q));
    // (A blocked step is not mended here. Moving its two points 1.5 m aside was tried - it
    // passes this test at the pillar on the plaza at 1335, -825, and the car then stops at
    // that pillar for 8 s; the grid search's line for the same hop is driven without a stop.)
    for (i, f) in floor.iter().enumerate() {
        if let (Some(f), true) = (f, i > 0 && i < last) {
            line[i].y = f.y + 0.2;
        }
    }
    let blocked = floor.windows(2).filter(|w| !open(w[0], w[1])).count();
    if blocked > 0 && std::env::var("MCLA_COURSE_DUMP").is_ok() {
        for (i, w) in floor.windows(2).enumerate() {
            if let (Some(p), Some(q)) = (w[0], w[1]) {
                if !clear(col, p, q) {
                    let d = (q - p).normalize_or_zero();
                    let hit = |h: f32| col.raycast(p + Vec3::Y * h, d, p.distance(q) + 0.6).map(|x| x.t);
                    log::info!("off-road: step {} of {} blocked: ({:.1}, {:.2}, {:.1}) -> ({:.1}, {:.2}, {:.1}); middle rays hit at {:?} / {:?}", i, last, p.x, p.y, p.z, q.x, q.y, q.z, hit(0.45), hit(1.15));
                }
            }
        }
    }
    Some(if blocked > 0 { Err(blocked) } else { Ok(line) })
}

/// A drivable line through the given points (first and last are kept). `solid`: things
/// standing around that are not part of the collision mesh (trees, posts).
pub fn plan(col: &WorldCollider, solid: &[Vec3], pts: &[Vec3]) -> Option<(Vec<Vec3>, bool)> {
    if pts.len() < 2 {
        return None;
    }
    if std::env::var("MCLA_OFFROAD_OLD").map_or(false, |v| v != "0") {
        return plan_grid(col, solid, pts);
    }
    let dump = std::env::var("MCLA_COURSE_DUMP").is_ok();
    let (a, b) = (pts[0], *pts.last().unwrap());
    match mesh().and_then(|m| plan_mesh(m, col, solid, pts)) {
        Some(Ok(line)) => {
            if dump {
                log::info!("off-road: ({:.1}, {:.1}, {:.1}) -> ({:.1}, {:.1}, {:.1}) over the mesh, {} points", a.x, a.y, a.z, b.x, b.y, b.z, line.len());
            }
            Some((line, true))
        }
        other => {
            let old = plan_grid(col, solid, pts);
            if dump || other.is_some() {
                let why = match other {
                    Some(Err(n)) => format!("the mesh's line is blocked at {n} steps"),
                    _ => "no way over the mesh".to_string(),
                };
                log::info!("off-road: ({:.1}, {:.1}, {:.1}) -> ({:.1}, {:.1}, {:.1}): {}; grid search {}", a.x, a.y, a.z, b.x, b.y, b.z, why, match old.as_ref() { Some((_, true)) => "through", Some(_) => "part of the way", None => "no way" });
            }
            old
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mcla_formats::rage::navsys::NavSys;

    fn data() -> Option<(String, NavMesh)> {
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let m = crate::assets::load_resource_file(&std::path::Path::new(&root).join("resources/city/sc/0x69d17d57.r26")).ok().and_then(|r| NavMesh::parse(&r))?;
        Some((root, m))
    }

    /// Every sample of a line lies on (or within a hand of) the mesh; `strict`: on a polygon
    /// a car may use.
    fn on_mesh(mesh: &NavMesh, line: &[Vec3], strict: bool) -> bool {
        line.windows(2).all(|w| {
            let n = (w[0].distance(w[1]) / 0.5).ceil().max(1.0) as usize;
            (0..=n).all(|k| mesh.nearest(w[0].lerp(w[1], k as f32 / n as f32), 0.3, 3.0, &|p| !strict || p.drivable()).is_some())
        })
    }

    /// Three hops of real races between route nodes away from the lane network (printed by
    /// the sweep below): the line exists, starts and ends where asked and stays on the mesh.
    #[test]
    fn mesh_lines_between_real_route_nodes() {
        let Some((_, mesh)) = data() else { return };
        for (a, b) in HOPS {
            let (a, b) = (Vec3::from(a), Vec3::from(b));
            let line = mesh_line(&mesh, &[], a, b).unwrap_or_else(|| panic!("no line {a:?} -> {b:?}"));
            let path = mesh.drive_path(a, b, SNAP, HALF).unwrap();
            let mut mats: Vec<(u8, usize)> = Vec::new();
            for p in &path.polys {
                let m = mesh.polys[*p as usize].material;
                match mats.last_mut() {
                    Some(l) if l.0 == m => l.1 += 1,
                    _ => mats.push((m, 1)),
                }
            }
            println!("line {a:?} -> {b:?}: strict {}, (material, polygons) {mats:?}, corners {:?}", path.strict, path.points);
            assert_eq!(*line.last().unwrap(), b);
            let len: f32 = std::iter::once(a).chain(line.iter().copied()).collect::<Vec<_>>().windows(2).map(|w| w[0].distance(w[1])).sum();
            assert!(len >= a.distance(b) - 0.5 && len < a.distance(b) * 4.0 + 150.0, "{len}");
            // between the two ends (which are the script's own points, up to SNAP off the mesh)
            assert!(on_mesh(&mesh, &line[..line.len() - 1], path.strict), "{a:?} -> {b:?} leaves the mesh");
            assert!(line.windows(2).all(|w| w[0].distance(w[1]) < STEP * 2.0 + SNAP), "a gap in {a:?} -> {b:?}");
            println!("line {a:?} -> {b:?}: {} points, {len:.0} m for {:.0} m straight", line.len(), a.distance(b));
        }
    }

    const HOPS: [([f32; 3], [f32; 3]); 3] = [([1358.5, 16.8, -806.4], [1321.7, 16.8, -824.7]), ([1321.7, 16.8, -824.7], [1313.0, 17.0, -846.2]), ([1339.6, 16.2, -776.5], [1321.7, 16.8, -824.7])];

    /// All race scripts with readable names: every hop of an opponents' route that has an end
    /// away from the lane network. Most must have a line over the mesh.
    #[test]
    fn mesh_lines_for_the_races_routes() {
        let Some((root, mesh)) = data() else { return };
        let Some(nav) = crate::assets::load_resource_file(&std::path::Path::new(&root).join("resources/city/sc/0x9960fcd8.r66")).ok().and_then(|r| NavSys::parse(&r).ok()) else { return };
        let base = std::path::Path::new(&root).join("script_obj/game/race");
        let (mut hops, mut found, mut strict, mut on, mut shown, mut examples) = (0, 0, 0, 0, 0, 0);
        let mut seen: Vec<(Vec3, Vec3)> = Vec::new();
        for d in std::fs::read_dir(&base).into_iter().flatten().flatten().filter(|e| e.path().is_dir()) {
            for t in std::fs::read_dir(d.path()).into_iter().flatten().flatten().filter(|e| e.path().is_dir()) {
                for f in std::fs::read_dir(t.path()).into_iter().flatten().flatten() {
                    let name = f.file_name().to_string_lossy().to_string();
                    let Some(stem) = name.strip_suffix(".sco") else { continue };
                    let id = format!("game/race/{}/{}/{}", d.file_name().to_string_lossy(), t.file_name().to_string_lossy(), stem);
                    let Some(c) = crate::racescript::course(&root, &id, None) else { continue };
                    let off = |p: Vec3| nav.nearest_link(p).map_or(true, |l| l.2 >= 9.0);
                    for r in &c.routes {
                        for k in r.start..(r.start + r.count).saturating_sub(1).min(c.nodes.len().saturating_sub(1)) {
                            let (a, b) = (c.nodes[k].0, c.nodes[k + 1].0);
                            if !(off(a) || off(b)) || seen.iter().any(|s| s.0.distance(a) < 0.5 && s.1.distance(b) < 0.5) {
                                continue;
                            }
                            seen.push((a, b));
                            hops += 1;
                            let Some(path) = mesh.drive_path(a, b, SNAP, HALF) else {
                                if shown < 12 {
                                    shown += 1;
                                    println!("line: no way {id}: {a:?} -> {b:?}");
                                }
                                continue;
                            };
                            strict += path.strict as usize;
                            if let Some(line) = mesh_line(&mesh, &[], a, b) {
                                found += 1;
                                let ok = on_mesh(&mesh, &line[..line.len() - 1], path.strict);
                                if ok && path.strict && off(a) && off(b) && (30.0..150.0).contains(&a.distance(b)) && examples < 8 {
                                    examples += 1;
                                    println!("line: for example {id}: {a:?} -> {b:?}, {:.0} m over {} polygons", path.length, path.polys.len());
                                }
                                on += ok as usize;
                                if !ok && shown < 12 {
                                    shown += 1;
                                    println!("line: off the mesh {id}: {a:?} -> {b:?}");
                                }
                            }
                        }
                    }
                }
            }
        }
        println!("line: {hops} hops with an end off the lane network: {found} with a line over the mesh ({strict} over drivable polygons only), {on} of them wholly on it");
        if hops > 0 {
            // (measured: 3,945 hops, 3,798 lines, 2,614 of them without plain pavement, 3,794 wholly on the mesh)
            assert!(found * 10 >= hops * 9, "{found} of {hops}");
            assert!(on * 200 >= found * 199, "{on} of {found}");
        }
    }
}
