//! Ambient pedestrians: the game's own character sets (`resources/character/ped_*_set`)
//! skinned on the CPU and driven by the original animations
//! (`resources/ambients/<set>/*.r189`: walk, idle, run; `resources/ambients/*.r198`:
//! the shared evade / angry clips listed in `tune/ambients/peds/mcAvoidAnimMgrData.xml`).
//!
//! They walk the sidewalk polygons of the original's navigation mesh (`city.xsm`, material
//! flag bit 0 - what `mcAmbientPed` / `mcSidewalkFilter` accept), from polygon to polygon
//! through the mesh's own links; the route choice is the port's (the original's path queries
//! are not decoded). Without the mesh file, or with `MCLA_PED_KERB=1`, they stroll along the
//! kerb of the outermost traffic lane as before. They jump out of the way of approaching
//! cars (they are never hit: `bCollideWithPlayer` is false in the tuning) and shout their
//! evade lines from the `ped_*` speech banks.

use crate::ambient::{xml_value, Distribution};
use crate::assets::load_resource_file;
use crate::physics::collision::WorldCollider;
use crate::render::{Frustum, Gpu};
use crate::skin::{blend_pose, Clip, SkinBuffer, SkinModel};
use glam::{Mat4, Quat, Vec3};
use hashbrown::{HashMap, HashSet};
use mcla_formats::rage::anim::{find_animations, Animation};
use mcla_formats::rage::navmesh::{NavLink, NavMesh, NavPoly, AI_SIDEWALK};
use mcla_formats::rage::navsys::NavSys;
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct PedType {
    female: bool,
    /// speech bank key, e.g. "fh_006"
    voice: String,
    model: SkinModel,
    walk: Clip,
    idle: Clip,
    evade: Vec<Clip>,
    angry: Vec<Clip>,
    /// the shared Pivot clips (`Mped_turnL/R_90/180`) with the yaw their root turns
    pivot: Vec<(Clip, f32)>,
    lod_switch: [f32; 2],
    /// the walk clip's root speed (m/s): its distance over its duration (A36[1] / A28[1])
    walk_speed: f32,
    /// `fMinTurningRadius` of the set's `mcAmbientType.xml` (type +160)
    turn_radius: f32,
}

impl PedType {
    fn clip(&self, s: State) -> (&Clip, bool) {
        match s {
            State::Walk => (&self.walk, true),
            State::Idle => (&self.idle, true),
            State::Evade(k) => (&self.evade[k], false),
            State::Angry(k) => (&self.angry[k], false),
            State::Turn(k) => (&self.pivot[k].0, false),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum State {
    Walk,
    Idle,
    Evade(usize),
    Angry(usize),
    /// turning on the spot with a Pivot clip
    Turn(usize),
}

struct Ped {
    ty: usize,
    key: (usize, u32),
    pos: Vec3,
    rot: Quat,
    link: usize,
    fwd: bool,
    lat: f32,
    state: State,
    time: f32,
    /// local pose of the previous state, cross-faded out over `blend` seconds
    from: Vec<(Quat, Vec3)>,
    blend: f32,
    cur: Vec<(Quat, Vec3)>,
    /// stands around instead of walking
    loiter: bool,
    calm: f32,
    seed: u32,
    /// what the walker wants to do at the corner it has reached
    plan: Option<Plan>,
    /// crossing the street: kerb point over there, then the link and offset to walk on
    cross: Option<(Vec3, usize, f32)>,
    /// seconds spent waiting at the kerb
    wait: f32,
    /// on the navigation mesh: the sidewalk polygon walked on (`NO_POLY`: kerb walker)
    poly: u32,
    /// direction of the stretch before
    travel: Vec3,
    /// on the navigation mesh: the points of the random path held (on the edges crossed)
    /// with the polygon behind each; empty once given up
    path: Vec<(Vec3, u32)>,
    /// 30 Hz frames since the last path request
    since_plan: u32,
    /// seconds still standing after appearing (the original's spawning state, flags 0x49)
    hold: f32,
    /// the sidewalk behaviour (mcAmbientBehaviorSidewalk, vtable 0x820da7f4):
    /// walking speed factor 0.85 .. 1.15 (beh +1336) and the speed now (ped +412)
    scale: f32,
    speed: f32,
    /// the rail's end point; `true` for a rail around something in the way
    rail: Option<(Vec3, bool)>,
    /// stop timer and the stops in a row (behaviour +500, +532 >> 3)
    stop: f32,
    stops: u32,
    /// sidestep or dodge: from, to, seconds into it, length
    slide: Option<(Vec3, Vec3, f32, f32)>,
    /// the last path was requested facing backwards: turn round to walk it
    turn_round: bool,
    /// yaw still to turn after the Pivot clip, and what follows the turn (angry after a dodge)
    turn_rest: f32,
    then_angry: bool,
    /// the car dodged last
    dodged: Option<Vec3>,
    /// a layout item's person (sub_826DFB08): its item, and its age and least age (8 + 4U s)
    item: Option<u32>,
    age: f32,
    life: f32,
    /// the length of the blend running (`blend` counts it down)
    blend_len: f32,
}

const NO_POLY: u32 = u32::MAX;
/// The original's frame: path requests are let through once per 30 Hz frame; spawning and
/// unspawning run on every other one (sub_826F79F0 alternates by frame parity).
const FRAME: f32 = 1.0 / 30.0;
/// The spawner's occlusion test (sub_826F0628): a sphere of 2 m, 1.5 m above the spawn
/// point, against the volumes seen from 1.75 m above the player (view 2, sub_822C1FA8).
const OCC_LIFT: f32 = 1.5;
const OCC_RADIUS: f32 = 2.0;
const OCC_EYE: f32 = 1.75;
/// The occluder range scale the city sets up (sub_822C2EA8 writes 10 to 0x827D2958).
const OCC_RANGE_SCALE: f32 = 10.0;

fn flat(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
}

/// The original's random walk over the sidewalk polygons (`navQueryPathRandom`, search
/// sub_827691B0, set up by sub_8276AB10). Up to 32 hops; at each hop every sidewalk
/// neighbour of the polygon weighs `L * max(((1 + cos) / 2)^2, 0.01)` - L the length of the
/// shared edge, cos between the heading and the way to the edge's middle - or 0.001 when it
/// is one of the last four polygons entered. The walk moves to the chosen edge's middle and
/// turns the heading half way towards that step. Returns per hop the shared edge (a, b) and
/// the polygon entered; on the 32nd hop the entry is that polygon's centre (a = b).
pub(crate) fn random_path(mesh: &NavMesh, start: u32, pos: Vec3, heading: Vec3, rand: &mut dyn FnMut() -> f32) -> Vec<(Vec3, Vec3, u32)> {
    let mut out = Vec::new();
    let mut ring = [NO_POLY; 4];
    let mut ring_i = 0;
    let mut cur = start;
    let mut p = pos;
    let mut h = flat(heading).normalize_or_zero();
    let mut cands: Vec<(f32, &NavLink)> = Vec::new();
    // starting off the pavement the filter is only a weight (x 1e-6 outside it), so the walk
    // finds its way back (sub_826FC258)
    let soft = !mesh.polys[start as usize].sidewalk();
    for hop in 0..32 {
        cands.clear();
        let mut total = 0.0;
        for l in mesh.links_of(cur) {
            let outside = !mesh.polys[l.poly as usize].sidewalk();
            if l.poly == cur || (outside && !soft) {
                continue;
            }
            let w = if ring.contains(&l.poly) {
                0.001
            } else {
                let len = l.a.distance(l.b);
                let len = if len > 0.0 { len } else { 0.1 };
                let d = flat((l.a + l.b) * 0.5 - p);
                if h == Vec3::ZERO {
                    len
                } else if d.length() <= 0.0 {
                    len * 0.01
                } else {
                    let c = (d.normalize().dot(h) + 1.0) * 0.5;
                    len * (c * c).max(0.01)
                }
            } * if outside { 1e-6 } else { 1.0 };
            total += w;
            cands.push((w, l));
        }
        if cands.is_empty() {
            break;
        }
        let mut x = rand() * total;
        let mut pick = cands[cands.len() - 1].1;
        for (w, l) in &cands {
            x -= w;
            if x < 0.0 {
                pick = l;
                break;
            }
        }
        // the shared edge in the corner order of the polygon left (the link records store
        // its ends either way round)
        let (mut ea, mut eb) = (pick.a, pick.b);
        let pts = mesh.polys[cur as usize].points();
        let at = |v: Vec3| pts.iter().position(|q| q.distance_squared(v) < 1e-4);
        if let (Some(ia), Some(ib)) = (at(ea), at(eb)) {
            if (ib + 1) % pts.len() == ia {
                std::mem::swap(&mut ea, &mut eb);
            }
        }
        let mid = (pick.a + pick.b) * 0.5;
        let step = flat(mid - p).normalize_or_zero();
        if step != Vec3::ZERO {
            h = (h + (step - h) * 0.5).normalize_or_zero();
        }
        p = mid;
        cur = pick.poly;
        ring[ring_i] = cur;
        ring_i = (ring_i + 1) % 4;
        if hop == 31 {
            let c = mesh.polys[cur as usize].center;
            out.push((c, c, cur));
        } else {
            out.push((ea, eb, cur));
        }
    }
    out
}

/// The pedestrian's own radius (ped +492, 0.3 on every ped live).
const PED_RADIUS: f32 = 0.3;
/// The awareness spheres (ped +2176 around the ped, +3392 ahead of it): radius 0.667.
const AWARE_RADIUS: f32 = 0.667;
/// The sphere ahead sits 0 .. 2.5 m in front, drawn anew for every test.
const AHEAD_MAX: f32 = 2.5;
/// Sphere refresh (sub_8279D2D8 via the manager): every 30 Hz frame each walker re-queries,
/// with this chance by its distance to the focus (within 25 m, 85 m, beyond), the one of its
/// three spheres the global frame counter points at (counter % 3): 0 the ground probe (r 2,
/// mask 0x800), 1 around the walker, 2 ahead of it. The hits only last that frame.
const AWARE_CHANCE: [(f32, f32); 2] = [(25.0, 0.4), (85.0, 0.15)];
const AWARE_CHANCE_FAR: f32 = 0.1;
/// Sidestep: 0.25 s, the push tried straight and 45 degrees either way (table 0x820da334).
const SLIDE_TIME: f32 = 0.25;
/// Clip blends are 8 frames (0x82830878).
const BLEND_S: f32 = 8.0 / 30.0;
/// Turning on the spot: at most pi/4 per 8 frames (sub_827B8678).
const TURN_RATE: f32 = 2.94524;
/// Rails: the first of the segments i, i+1, i+2, i-1, i-2 whose middle is more than 0.5 m
/// away and at most 72 degrees round; else segment i with up to 216 (sub_827983B8).
const RAIL_TURN: f32 = 1.25664;
const RAIL_TURN_LAST: f32 = 3.76991;
const RAIL_MIN: f32 = 0.5;
/// A rail around something in the way: at most 130 degrees round (sub_827A14C0); the way
/// to it swept by a capsule of the ped's radius 0.6 - 1.2 m up (sub_8279D9D0, mask 0x383e:
/// furniture, cars, peds and buildings, the walker being passed included).
const AVOID_TURN: f32 = 2.26893;
const AVOID_CLEAR: f32 = PED_RADIUS;
/// Speed follows the wanted speed with a 0.1 s lag (sub_8279EF40, sub_827966C0).
const SPEED_LAG: f32 = 0.1;

fn rotate_y(v: Vec3, a: f32) -> Vec3 {
    let (s, c) = a.sin_cos();
    Vec3::new(v.x * c + v.z * s, v.y, -v.x * s + v.z * c)
}

/// Signed yaw from direction `a` to direction `b` (both flat), in the port's yaw sense.
fn yaw_between(a: Vec3, b: Vec3) -> f32 {
    wrap_angle((-b.x).atan2(-b.z) - (-a.x).atan2(-a.z))
}

/// Street furniture as the walkers see it: footprint box (centre, its flat x axis, half
/// sizes along x and z). A walker counts as a box of its radius.
pub type Obst = (Vec3, Vec3, glam::Vec2);

/// Distance in the ground plane from `q` to the box (0 inside).
fn box_dist(o: &Obst, q: Vec3) -> f32 {
    let d = flat(q - o.0);
    let z = Vec3::new(-o.1.z, 0.0, o.1.x);
    let (lx, lz) = ((d.dot(o.1).abs() - o.2.x).max(0.0), (d.dot(z).abs() - o.2.y).max(0.0));
    (lx * lx + lz * lz).sqrt()
}

fn box_corners(o: &Obst) -> [Vec3; 4] {
    let z = Vec3::new(-o.1.z, 0.0, o.1.x);
    let (x, z) = (o.1 * o.2.x, z * o.2.y);
    [o.0 + x + z, o.0 + x - z, o.0 - x - z, o.0 - x + z]
}

fn ped_box(q: Vec3) -> Obst {
    (q, Vec3::X, glam::Vec2::splat(PED_RADIUS))
}

/// What a sphere touches: other walkers and street furniture, the nearest first; (its box,
/// walker index or `usize::MAX`).
fn sphere_hit(c: Vec3, r: f32, me: usize, peds: &[Vec3], props: &[Obst], skip_peds: bool) -> Option<(Obst, usize)> {
    let mut best: Option<(f32, Obst, usize)> = None;
    if !skip_peds {
        for (k, q) in peds.iter().enumerate() {
            let d = box_dist(&ped_box(*q), c);
            if k != me && (q.y - c.y).abs() < 2.0 && d < r && best.map_or(true, |b| d < b.0) {
                best = Some((d, ped_box(*q), k));
            }
        }
    }
    for o in props {
        let d = box_dist(o, c);
        if d < r && best.map_or(true, |b| d < b.0) {
            best = Some((d, *o, usize::MAX));
        }
    }
    best.map(|b| (b.1, b.2))
}

/// Is the way from `a` to `b` clear by `clear` metres (peds unless `skip_peds`, furniture;
/// the walker `ignore` and the box centred at `ignore_box` - what is being passed - are left
/// out)? With `away`, something the way leads away from does not count (the sidestep's
/// probe starts touching what the walker is pushed out of); the detour's sweep has no such
/// test (sub_8279D9D0).
#[allow(clippy::too_many_arguments)]
fn way_clear(a: Vec3, b: Vec3, clear: f32, me: usize, ignore: usize, ignore_box: Option<Vec3>, peds: &[Vec3], props: &[Obst], col: Option<&WorldCollider>, skip_peds: bool, away: bool) -> bool {
    let n = (flat(b - a).length() / 0.2).ceil().max(1.0) as usize;
    // the city's own bound (buildings, walls; the ground stays below the probe), swept 0.6 -
    // 1.2 m up
    let lift = Vec3::Y * (0.6 + clear);
    if col.is_some_and(|c| (0..=n).any(|k| c.sphere(a.lerp(b, k as f32 / n as f32) + lift, clear).is_some())) {
        return false;
    }
    let seg = |o: &Obst| {
        if away && box_dist(o, b) >= box_dist(o, a) {
            return f32::MAX;
        }
        (0..=n).map(|k| box_dist(o, a.lerp(b, k as f32 / n as f32))).fold(f32::MAX, f32::min)
    };
    if !skip_peds && peds.iter().enumerate().any(|(k, q)| k != me && k != ignore && (q.y - a.y).abs() < 2.0 && seg(&ped_box(*q)) < clear) {
        return false;
    }
    !props.iter().any(|o| Some(o.0) != ignore_box && seg(o) < clear)
}

/// The next rail along the path from `pos` facing `fwd` (sub_827983B8): to the middle of
/// the path segment nearest to the walker, or of one of its neighbours.
fn next_rail(path: &[(Vec3, u32)], pos: Vec3, fwd: Vec3) -> Option<Vec3> {
    if path.len() < 2 {
        return None;
    }
    let n = path.len() - 1;
    let mut i = 0;
    let mut best = f32::MAX;
    for k in 0..n {
        let (a, b) = (path[k].0, path[k + 1].0);
        let ab = flat(b - a);
        let t = (flat(pos - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
        let d = flat(pos - (a + ab * t)).length_squared();
        if d < best {
            best = d;
            i = k;
        }
    }
    let try_seg = |k: isize, max_turn: f32| -> Option<Vec3> {
        if k < 0 || k as usize >= n {
            return None;
        }
        let m = (path[k as usize].0 + path[k as usize + 1].0) * 0.5;
        let d = flat(m - pos);
        (d.length() > RAIL_MIN && yaw_between(fwd, d.normalize()).abs() <= max_turn).then_some(m)
    };
    let i = i as isize;
    [i, i + 1, i + 2, i - 1, i - 2].into_iter().find_map(|k| try_seg(k, RAIL_TURN)).or_else(|| try_seg(i, RAIL_TURN_LAST))
}

/// The wait timer (sub_8279A8C8) from the idle clip's length `v`.
fn stop_time(v: f32, rand: &mut dyn FnMut() -> f32) -> f32 {
    if v <= 0.0 {
        2.0 + 2.0 * rand()
    } else if v < 5.0 {
        ((rand() * 3.0) as u32 % 3 + 1) as f32 * v - BLEND_S
    } else {
        v - BLEND_S
    }
}

/// A car about to run the walker over (sub_8279B538): its path over the next second in
/// four steps of 0.25 s passing within its half width and the walker's radius. Returns the
/// car's position and the way out (away from the nearest point of its path).
fn car_threat(pos: Vec3, threats: &[Threat]) -> Option<(Vec3, Vec3, f32)> {
    for t in threats {
        if (t.pos.y - pos.y).abs() > 5.0 || t.vel.length_squared() < 1e-4 {
            continue;
        }
        // half width of a car side on (the original picks the half extent of the side it
        // comes at; not decoded - the half width of an average car)
        let reach = 1.0 + PED_RADIUS;
        for k in 1..=4 {
            let (a, b) = (t.pos + t.vel * (0.25 * (k - 1) as f32), t.pos + t.vel * (0.25 * k as f32));
            let ab = flat(b - a);
            let s = flat(pos - a).dot(ab) / ab.length_squared().max(1e-6);
            if s <= 0.0 || s >= 1.0 {
                continue;
            }
            let near = a + ab * s;
            let off = flat(pos - near);
            if off.length() < reach {
                let away = if off.length() > 1e-3 { off.normalize() } else { Vec3::new(-t.vel.z, 0.0, t.vel.x).normalize_or_zero() };
                return Some((t.pos, away, t.vel.length()));
            }
        }
    }
    None
}

/// One frame of a walker on the navigation mesh in the original's sidewalk behaviour
/// (mcAmbientBehaviorSidewalk::Update sub_8279F198): stops (sub_8279A8C8 / 8279AA18),
/// sidesteps (sub_8279C038), rails round things in the way (sub_82799538), the path's
/// rails (sub_827983B8, sub_82798F18), car dodges (sub_8279B538) and the turn round of a
/// reversed path (sub_8279C198). Moves `p` and sets its state; returns the walk clip's
/// playback rate.
#[allow(clippy::too_many_arguments)]
fn sidewalk_step(p: &mut Ped, ty: &PedType, me: usize, peds: &[Vec3], props: &[Obst], col: Option<&WorldCollider>, threats: &[Threat], due: u8, dt: f32, rand: &mut dyn FnMut() -> f32, evades: &mut u32, speech: &mut Vec<Speech>) -> f32 {
    let fwd = flat(p.rot * -Vec3::Z).normalize_or(Vec3::Z);
    let set_state = |p: &mut Ped, s: State| {
        if p.state != s {
            p.from = p.cur.clone();
            p.blend = 0.25;
            p.time = 0.0;
            p.state = s;
        }
    };
    p.hold = (p.hold - dt).max(0.0);
    // clips playing (dodge, angry, turn): the caller moves the walker by their root
    if matches!(p.state, State::Evade(_) | State::Angry(_) | State::Turn(_)) {
        return 1.0;
    }
    // a car coming: dodge out of its way with an evade clip
    if p.calm <= 0.0 && !ty.evade.is_empty() && p.hold <= 0.0 {
        if let Some((car, away, speed)) = car_threat(p.pos, threats) {
            let from_front = fwd.dot(flat(car - p.pos).normalize_or_zero()) > 0.4;
            let mut best = (f32::MIN, 0usize);
            for (k, c) in ty.evade.iter().enumerate() {
                if c.front && !from_front {
                    continue;
                }
                let s = (p.rot * c.travel.normalize_or_zero()).dot(away) + rand() * 0.5;
                if s > best.0 {
                    best = (s, k);
                }
            }
            p.from = p.cur.clone();
            p.blend = 0.15;
            p.state = State::Evade(best.1);
            p.time = 0.0;
            p.slide = None;
            p.rail = None;
            p.path.clear();
            p.dodged = Some(car);
            *evades += 1;
            speech.push(Speech { pos: p.pos, voice: ty.voice.clone(), female: ty.female, fast: ty.evade[best.1].fast || speed * speed > 244.77 });
            return 1.0;
        }
    }
    if p.hold > 0.0 {
        set_state(p, State::Idle);
        p.speed = 0.0;
        return 1.0;
    }
    // the awareness spheres re-queried this frame (`due`: bit 1 around the walker, bit 2
    // ahead of it, 0 .. 2.5 m drawn anew); their hits are acted on in the same frame
    let (query_around, query) = (due & 2 != 0, due & 4 != 0);
    let ahead = p.pos + fwd * (AHEAD_MAX * rand());
    // sliding aside
    if let Some((a, b, t, len)) = p.slide {
        let t = t + dt;
        p.pos = a.lerp(b, (t / len).min(1.0));
        p.speed = a.distance(b) / len;
        p.slide = if t >= len { None } else { Some((a, b, t, len)) };
        set_state(p, State::Walk);
        return p.speed / ty.walk_speed.max(0.1);
    }
    // a reversed path: turn round as it is taken (sub_8279F260; a Pivot clip beyond
    // 45 degrees), which ends the wait
    if p.turn_round {
        p.turn_round = false;
        let want = yaw_between(fwd, -fwd);
        if want.abs() > std::f32::consts::FRAC_PI_4 && !ty.pivot.is_empty() {
            let k = (0..ty.pivot.len()).min_by(|&a, &b| wrap_angle(want - ty.pivot[a].1).abs().total_cmp(&wrap_angle(want - ty.pivot[b].1).abs())).unwrap();
            p.turn_rest = wrap_angle(want - ty.pivot[k].1);
            p.then_angry = false;
            p.stop = 0.0;
            p.speed = 0.0;
            set_state(p, State::Turn(k));
            return 1.0;
        }
    }
    // stopped: wait; when the time is up, still in the way -> stop again, two or more stops in
    // a row -> give up the path (the next one may be asked for backwards)
    let mut expired = false;
    if p.stop > 0.0 {
        p.stop -= dt;
        if p.stop > 0.0 {
            set_state(p, State::Idle);
            p.speed = 0.0;
            return 1.0;
        }
        expired = true;
    }
    // something touching the walker: sidestep, which also drops the path
    if let Some((o, who)) = sphere_hit(p.pos, AWARE_RADIUS, me, peds, props, false).filter(|_| query_around) {
        let rel = flat(p.pos - o.0);
        let dist = box_dist(&o, p.pos);
        if std::env::var("MCLA_PED_DUMP").is_ok() {
            log::info!("ped touch {} {} depth {:.2}", p.key.1, if who == usize::MAX { "prop" } else { "ped" }, AWARE_RADIUS - dist);
        }
        let (n, depth) = if rel.length() > 1e-3 && dist > 1e-3 { (rel.normalize(), AWARE_RADIUS - dist) } else { (fwd, PED_RADIUS) };
        for a in [0.0f32, -std::f32::consts::FRAC_PI_4, std::f32::consts::FRAC_PI_4] {
            let d = rotate_y(n, a);
            let off = d * (depth + 0.05) / a.cos();
            if way_clear(p.pos, p.pos + off, PED_RADIUS, me, usize::MAX, None, peds, props, col, true, true) {
                // (the heading stays: the recorded walks keep their line through sidesteps)
                p.slide = Some((p.pos, p.pos + off, 0.0, SLIDE_TIME));
                p.rail = None;
                p.path.clear();
                return 1.0;
            }
        }
    }
    // something ahead (sub_8279AA18, run every frame the walker is not waiting, on a detour
    // rail too): a rail round it, or stop. The candidates (sub_827A14C0): the corner of the
    // box furthest left as seen from the walker, pushed out to the left by half the walker's
    // width and 0.25 (0.55); for furniture also the corner furthest right pushed right; a
    // walker in the way is only ever passed on the left (sub_8279AA18 clears args +44 bit 0x40
    // for type 0x20). Each is reached by a turn of at most 130 degrees (turning radius 0.1:
    // arcs too short to sample) and swept by the capsule all the way; the shortest wins.
    let hit = if query { sphere_hit(ahead, AWARE_RADIUS, me, peds, props, false) } else { None };
    if let Some((o, who)) = hit {
        let corners = box_corners(&o);
        let d0 = flat(o.0 - p.pos);
        let side = |c: Vec3| {
            let c = flat(c - p.pos).normalize_or_zero();
            d0.x * c.z - d0.z * c.x
        };
        let left = corners.iter().copied().min_by(|a, b| side(*a).total_cmp(&side(*b))).unwrap();
        let right = corners.iter().copied().max_by(|a, b| side(*a).total_cmp(&side(*b))).unwrap();
        let push = 0.5 * (2.0 * PED_RADIUS) + 0.25;
        let mut pass: Option<(f32, Vec3)> = None;
        let mut why = String::new();
        let both = who == usize::MAX;
        for (corner, sgn) in [(left, 1.0f32), (right, -1.0)].into_iter().take(if both { 2 } else { 1 }) {
            let v = flat(corner - p.pos);
            let pp = corner + Vec3::new(v.z, 0.0, -v.x).normalize_or_zero() * (sgn * push);
            let d = flat(pp - p.pos);
            if d.length() < 1e-3 || yaw_between(fwd, d.normalize()).abs() > AVOID_TURN {
                why.push('T');
                continue;
            }
            if way_clear(p.pos, pp, AVOID_CLEAR, me, usize::MAX, None, peds, props, col, false, false) && pass.map_or(true, |b| d.length() <= b.0) {
                pass = Some((d.length(), pp));
                why.push('+');
            } else {
                why.push('C');
            }
        }
        if std::env::var("MCLA_PED_DUMP").is_ok() {
            log::info!("ped avoid {} {} {} ahead {:.2}", p.key.1, if who == usize::MAX { "prop" } else { "ped" }, why, flat(o.0 - p.pos).dot(fwd));
        }
        match pass {
            Some((_, pp)) => {
                p.rail = Some((pp, true));
                p.stops = 0;
            }
            None => {
                if std::env::var("MCLA_PED_DUMP").is_ok() {
                    log::info!("ped stop {} by {} at {:.1} m, half {:.2} {:.2} centre {:.1} {:.1}", p.key.1, if who == usize::MAX { "prop" } else { "ped" }, box_dist(&o, p.pos), o.2.x, o.2.y, o.0.x, o.0.z);
                }
                p.stop = stop_time(ty.idle.anim.duration, rand);
                p.stops += 1;
                if expired && p.stops > 1 {
                    p.rail = None;
                    p.path.clear();
                }
                set_state(p, State::Idle);
                p.speed = 0.0;
                return 1.0;
            }
        }
    } else {
        // nothing in the slot: the stops in a row start again
        p.stops = 0;
    }
    // along the rails
    let mut step = {
        let want = ty.walk_speed * p.scale;
        p.speed += (want - p.speed) * (dt / SPEED_LAG).min(1.0);
        p.speed * dt
    };
    let mut heading = fwd;
    for _ in 0..3 {
        if p.rail.is_none() {
            p.rail = next_rail(&p.path, p.pos, heading).map(|m| (m, false));
            if p.rail.is_none() {
                // the path is used up: drop it and stand until the next one
                p.path.clear();
                break;
            }
        }
        let (target, _) = p.rail.unwrap();
        let to = flat(target - p.pos);
        let dist = to.length();
        // an arc of the turning radius, then straight on
        let turn = yaw_between(heading, to.normalize_or(heading));
        let a = turn.clamp(-step / ty.turn_radius.max(0.01), step / ty.turn_radius.max(0.01));
        heading = rotate_y(heading, a);
        if dist <= step && turn.abs() < 0.2 {
            p.pos = Vec3::new(target.x, p.pos.y, target.z);
            step -= dist;
            p.rail = None;
            continue;
        }
        p.pos += heading * step;
        break;
    }
    if p.rail.is_none() && p.path.is_empty() {
        set_state(p, State::Idle);
        p.speed = 0.0;
        return 1.0;
    }
    p.rot = Quat::from_rotation_y((-heading.x).atan2(-heading.z));
    set_state(p, State::Walk);
    p.speed / ty.walk_speed.max(0.1)
}

/// The pedestrian spawn candidates around `focus` (sub_826F05C8 -> sub_826F97F8): the
/// sidewalk polygons a sphere of radius `r` touches, the first 128 in the order of the
/// level's chunks and their quadtrees, sorted (`NavMesh::sphere_query`), with their areas.
pub(crate) fn spawn_candidates(mesh: &NavMesh, focus: Vec3, r: f32) -> Vec<(u32, f32)> {
    let found = mesh.sphere_query(focus, r, 128, |_, q| q.sidewalk() && q.nv >= 3 && q.closest(focus.x, focus.z).distance(focus) <= r);
    found.into_iter().map(|i| (i, mesh.polys[i as usize].area)).collect()
}

/// A random point of a polygon as the original picks it (sub_826F9A10): a triangle of the
/// fan from the first corner, chosen by area with `r1`, the rest of `r1` across it and
/// sqrt(`r2`) from the first corner towards the far edge.
pub(crate) fn random_point(q: &NavPoly, r1: f32, r2: f32) -> Vec3 {
    let v = q.points();
    if v.len() < 3 {
        return q.center;
    }
    let n = v.len() - 2;
    let mut w = [0.0f32; 2];
    for k in 0..n {
        w[k] = (v[k + 1] - v[0]).cross(v[k + 2] - v[0]).length();
    }
    let total: f32 = w[..n].iter().sum();
    if total <= 0.0 {
        return q.center;
    }
    let mut acc = 0.0;
    let mut k = 0;
    while k < n {
        acc += w[k] / total;
        if acc >= r1 {
            break;
        }
        k += 1;
    }
    let k = k.min(n - 1);
    let t = ((r1 - (acc - w[k] / total)) / (w[k] / total)).clamp(0.0, 1.0);
    let s = r2.sqrt();
    v[0] * (1.0 - s) + v[k + 1] * (s * (1.0 - t)) + v[k + 2] * (s * t)
}

#[derive(Clone, Copy)]
enum Plan {
    /// carry on along the next block (over the side street)
    Straight(usize, f32),
    /// over to the opposite pavement of the own street
    Cross(Vec3, usize, f32),
}

/// A moving car the pedestrians have to watch out for.
#[derive(Clone, Copy)]
pub struct Threat {
    pub pos: Vec3,
    pub vel: Vec3,
}

pub struct Speech {
    pub pos: Vec3,
    pub voice: String,
    pub female: bool,
    pub fast: bool,
}

/// `tune/ambients/density_tuning.xml` (the original's density manager, vtable 0x820c39c4).
struct Tuning {
    /// within this a spawn point must not be in view (sub_826F0628)
    spawn_inner: f32,
    /// radius of the sphere whose sidewalk polygons are spawn candidates
    spawn_outer: f32,
    /// unspawned further than this ahead, half of it behind (sub_826F49B8)
    unspawn: f32,
    /// pedestrians per square metre of candidate sidewalk
    density: f32,
    /// width of the band along the view direction peds live in
    box_width: f32,
    ped_spacing: f32,
    type_spacing: f32,
    /// view cone half angle (degrees) by speed (mph)
    fov_mph: [f32; 2],
    fov_deg: [f32; 2],
}

/// A pedestrian type's files as read: its character set and its own clips.
type TypeRead = (mcla_formats::rage::character::Character, Vec<Animation>);

fn read_type(model: &Path, anims: &Path) -> Option<TypeRead> {
    Some((SkinModel::read(model)?, find_animations(&load_resource_file(anims).ok()?)))
}

pub struct Peds {
    /// the ambient manager's front-end mode (sub_826D4C90, the start screen): the spawner has no
    /// visibility test (sub_826F3038, 0x826F36BC); the focus and view are the camera's
    pub frontend: bool,
    /// spawner frames still to run at once (the front-end mode's loading wait, traffic.rs
    /// `FRONT_END_PREROLL`)
    preroll: u32,
    districts: Distribution,
    hour: f32,
    dirs: Vec<(String, PathBuf, PathBuf)>,
    tune_dir: PathBuf,
    types: HashMap<usize, Option<PedType>>,
    /// types whose files are being read on a worker thread (`read_ahead`)
    reading: HashMap<usize, std::thread::JoinHandle<Option<TypeRead>>>,
    /// a second place and hour whose types are read ahead (`read_ahead_also`)
    ahead_also: Option<(Vec3, f32)>,
    avoid: Vec<(Arc<Animation>, bool, bool, u8)>,
    list: Vec<Ped>,
    active: HashSet<(usize, u32)>,
    curb: HashMap<usize, Option<f32>>,
    tune: Tuning,
    timer: f32,
    filled: bool,
    rng: u32,
    pub speech: Vec<Speech>,
    /// streets crossed so far (diagnostics)
    pub crossings: u32,
    /// evades started so far (diagnostics)
    pub evades: u32,
    evade_radius: f32,
    voices: Option<HashMap<String, String>>,
    /// the speech banks' scan, started ahead on a thread (`scan_voices_ahead`)
    voices_scan: Option<std::thread::JoinHandle<HashMap<String, String>>>,
    buf: SkinBuffer,
    /// at night: the lights the walkers' picker lays over their grid lamps, and the 30 Hz frames
    /// since the last frame (main.rs sets them each frame; None by day - figlight.rs)
    pub night_lights: Option<(crate::figlight::Dynamic, f32)>,
    /// each lit walker's eased records (the original's walker + 64)
    fig_lights: HashMap<(usize, u32), crate::figlight::FigureLight>,
    /// each walker's LOD of this frame by the main view (`pick_lods`; -1 not drawn)
    lod_pick: HashMap<(usize, u32), i8>,
    pub enabled: bool,
    /// the original's navigation mesh; without it the pedestrians follow the kerbs
    mesh: Option<NavMesh>,
    /// connected piece of pavement of every mesh polygon
    island: Vec<u32>,
    /// the player's car: flattened forward direction and speed (mph)
    view_dir: Vec3,
    speed_mph: f32,
    /// time not yet run in 30 Hz frames, and the frames run (their parity)
    step: f32,
    frame_no: u32,
    data_dir: String,
    /// next pedestrian to be offered the frame's path request
    plan_cursor: usize,
    /// this frame's spawn candidates (polygon, area) and the wanted count (diagnostics)
    cands: Vec<(u32, f32)>,
    pub target: f32,
    next_id: u32,
    /// MCLA_PED_TRACE: rows of every pedestrian twice a second
    trace: Option<(std::io::BufWriter<std::fs::File>, f32, f32)>,
}

fn hash(a: u32, b: u32) -> u32 {
    let mut h = a.wrapping_mul(0x9E37_79B9) ^ b.wrapping_mul(0x85EB_CA6B).rotate_left(13);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846C_A68B);
    h ^ (h >> 16)
}

/// A person type's name (the character sets `ped_*_set` / `drv_*_set`), as the ambient type list
/// holds them next to the car types (sub_826D5390).
pub fn is_person_type(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.ends_with("_set") && (n.starts_with("ped_") || n.starts_with("drv_"))
}

fn wrap_angle(mut d: f32) -> f32 {
    while d > std::f32::consts::PI {
        d -= std::f32::consts::TAU;
    }
    while d < -std::f32::consts::PI {
        d += std::f32::consts::TAU;
    }
    d
}

impl Peds {
    pub fn new() -> Peds {
        Peds {
            frontend: false,
            preroll: 0,
            districts: Distribution::default(),
            hour: 12.0,
            dirs: Vec::new(),
            tune_dir: PathBuf::new(),
            types: HashMap::new(),
            reading: HashMap::new(),
            ahead_also: None,
            avoid: Vec::new(),
            list: Vec::new(),
            active: HashSet::new(),
            curb: HashMap::new(),
            tune: Tuning { spawn_inner: 95.0, spawn_outer: 140.0, unspawn: 150.0, density: 0.011, box_width: 100.0, ped_spacing: 1.0, type_spacing: 20.0, fov_mph: [25.0, 45.0], fov_deg: [89.9, 45.0] },
            timer: 0.0,
            filled: false,
            rng: 0x1234_5678,
            speech: Vec::new(),
            evades: 0,
            crossings: 0,
            evade_radius: std::env::var("MCLA_PED_EVADE").ok().and_then(|v| v.parse().ok()).unwrap_or(2.3),
            voices: None,
            voices_scan: None,
            buf: SkinBuffer::default(),
            night_lights: None,
            fig_lights: HashMap::new(),
            lod_pick: HashMap::new(),
            enabled: true,
            mesh: None,
            island: Vec::new(),
            view_dir: -Vec3::Z,
            speed_mph: 0.0,
            step: 0.0,
            frame_no: 0,
            data_dir: String::new(),
            plan_cursor: 0,
            cands: Vec::new(),
            target: 0.0,
            next_id: 0,
            trace: std::env::var("MCLA_PED_TRACE").ok().and_then(|f| std::fs::File::create(f).ok()).map(|f| (std::io::BufWriter::new(f), 0.0, 0.0)),
        }
    }

    /// The types around `at` at an hour read ahead too (on the start screen: the player's car and the hour given back
    /// after it, where START takes the focus - its first tick read 8 types, 31 - 207 ms; docs/PERFORMANCE.md 2026-10-26).
    pub fn read_ahead_also(&mut self, at: Option<(Vec3, f32)>) {
        self.ahead_also = at;
    }

    /// The ambient manager's front-end mode on or off; switched on, the frames of the loading wait
    /// run first (traffic.rs `FRONT_END_PREROLL`).
    pub fn set_frontend(&mut self, on: bool) {
        if on && !self.frontend {
            self.preroll = crate::vehicles::traffic::FRONT_END_PREROLL;
        }
        self.frontend = on;
    }

    /// The player's view the spawner works with: the car's forward direction and speed.
    pub fn set_view(&mut self, forward: Vec3, speed_mph: f32) {
        let f = flat(forward).normalize_or_zero();
        if f != Vec3::ZERO {
            self.view_dir = f;
        }
        self.speed_mph = speed_mph;
    }

    fn rand(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.rng >> 8) as f32 / 16777216.0
    }

    /// Index the pedestrian sets and load the shared evade animations.
    pub fn load(&mut self, data_dir: &str) {
        let root = Path::new(data_dir);
        self.tune_dir = root.join("tune/ambients/peds");
        self.data_dir = data_dir.to_string();
        if let Ok(t) = std::fs::read_to_string(root.join("tune/ambients/density_tuning.xml")) {
            let g = |tag: &str, d: f32| xml_value(&t, tag).unwrap_or(d);
            self.tune = Tuning {
                spawn_inner: g("fPedSpawnInnerRadius", 95.0),
                spawn_outer: g("fPedSpawnOuterRadius", 140.0),
                unspawn: g("fPedUnspawnOuterRadius", 150.0),
                density: g("fPedDensity", 0.011),
                box_width: g("fPedBoxWidth", 100.0),
                ped_spacing: g("fMinSpawnPedSpacing", 1.0),
                type_spacing: g("fMinSpawnPedTypeSpacing", 20.0),
                fov_mph: [g("fFOVDegreesLowMPH", 25.0), g("fFOVDegreesHighMPH", 45.0)],
                fov_deg: [g("fFOVDegreesAtLowSpeed", 89.9), g("fFOVDegreesAtHighSpeed", 45.0)],
            };
        }
        let first = |dir: &Path, ext: &str| -> Option<PathBuf> {
            let mut v: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e == ext).unwrap_or(false)).collect();
            v.sort();
            v.into_iter().next()
        };
        let mut names: Vec<String> = std::fs::read_dir(root.join("resources/ambients")).map(|d| d.flatten().filter_map(|e| e.file_name().into_string().ok()).collect()).unwrap_or_default();
        names.sort();
        for n in names {
            if !n.starts_with("ped_") {
                continue;
            }
            let (Some(model), Some(anims)) = (first(&root.join("resources/character").join(&n), "r6"), first(&root.join("resources/ambients").join(&n), "r189")) else { continue };
            self.dirs.push((n, model, anims));
        }
        // shared evade / angry clips, classified by the game's own list
        let mut class: HashMap<String, (bool, bool, u8)> = HashMap::new();
        if let Ok(t) = std::fs::read_to_string(self.tune_dir.join("mcAvoidAnimMgrData.xml")) {
            for item in t.split("<Item>").skip(1) {
                let field = |tag: &str| -> String {
                    let open = format!("<{tag}>");
                    let Some(i) = item.find(&open) else { return String::new() };
                    let rest = &item[i + open.len()..];
                    rest.find('<').map(|e| rest[..e].trim().to_string()).unwrap_or_default()
                };
                let name = field("Name").rsplit('/').next().unwrap_or("").trim_end_matches(".anim").to_ascii_lowercase();
                let kind = match field("AvoidType").as_str() {
                    "AvoidMale" | "AvoidFemale" => 0,
                    "AngryMale" | "AngryFemale" => 1,
                    "Pivot" => 3,
                    _ => 2,
                };
                class.insert(name, (field("AudioType") == "Fast", field("ApproachDir") == "Front", kind));
            }
        }
        if let Ok(res) = load_resource_file(&root.join("resources/ambients/0x364e4eea.r198")) {
            for a in find_animations(&res) {
                let (fast, front, kind) = class.get(&a.short()).copied().unwrap_or((false, false, 2));
                self.avoid.push((Arc::new(a), fast, front, kind));
            }
        }
        self.districts = Distribution::load(root, "ped_");
        // the navigation mesh (city.xsm); MCLA_PED_KERB=1 keeps the kerb walkers
        if std::env::var("MCLA_PED_KERB").is_err() {
            self.mesh = load_resource_file(&root.join("resources/city/sc/0x69d17d57.r26")).ok().and_then(|r| NavMesh::parse(&r));
            match &self.mesh {
                Some(m) => {
                    let (island, n) = m.sidewalk_islands();
                    log::info!("peds: nav mesh {} polygons, {} sidewalk in {} islands", m.polys.len(), island.iter().filter(|&&i| i != u32::MAX).count(), n);
                    self.island = island;
                }
                None => log::warn!("peds: no navigation mesh, walking the kerbs"),
            }
        }
        log::info!("peds: {} types, {} shared clips, {} districts", self.dirs.len(), self.avoid.len(), self.districts.types.len());
    }

    fn build_type(&mut self, gpu: &mut Gpu, ti: usize) -> Option<PedType> {
        let t0 = std::time::Instant::now();
        let read = match self.reading.remove(&ti) {
            Some(h) => h.join().ok().flatten(),
            None => read_type(&self.dirs[ti].1, &self.dirs[ti].2),
        };
        let ms_read = t0.elapsed().as_secs_f32() * 1000.0;
        let (name, path, _) = &self.dirs[ti];
        let (c, own) = read?;
        let model = SkinModel::from_character(gpu, name, path, c)?;
        let own: Vec<Arc<Animation>> = own.into_iter().map(Arc::new).collect();
        log::info!("peds: type {} built in {:.0} ms ({:.0} reading the files)", name, t0.elapsed().as_secs_f32() * 1000.0, ms_read);
        let pick = |what: &str| own.iter().find(|a| a.short().contains(what));
        let walk = Clip::bind(pick("walk")?, &model.ids, false, false);
        let idle = Clip::bind(pick("idle").or(pick("walk"))?, &model.ids, false, false);
        // "ped_fh_006_set" -> female, voice "fh_006"
        let core = name.trim_start_matches("ped_").trim_end_matches("_set").to_string();
        let female = core.trim_start_matches("sc_").starts_with('f');
        let gender = if female { "fped" } else { "mped" };
        let mut evade = Vec::new();
        let mut angry = Vec::new();
        // Pivot clips are chosen without regard to sex (sub_826F7FA8)
        let mut pivot = Vec::new();
        for (a, fast, front, kind) in &self.avoid {
            if *kind == 3 {
                let c = Clip::bind(a, &model.ids, *fast, *front);
                let q = c.mover(c.last_frame()).1 * c.mover(0.0).1.inverse();
                let yaw = wrap_angle(2.0 * q.y.atan2(q.w));
                pivot.push((c, yaw));
                continue;
            }
            if !a.short().starts_with(gender) {
                continue;
            }
            match kind {
                0 => evade.push(Clip::bind(a, &model.ids, *fast, *front)),
                1 => angry.push(Clip::bind(a, &model.ids, *fast, *front)),
                _ => {}
            }
        }
        let mut lod_switch = [40.0, 80.0];
        if let Ok(t) = std::fs::read_to_string(self.tune_dir.join(name).join("mcAmbientRenderType.xml")) {
            let s = xml_value(&t, "fLodSwitchScale").unwrap_or(1.0);
            lod_switch = [xml_value(&t, "fHighToMedSwitch").unwrap_or(40.0) * s, xml_value(&t, "fMedToLowSwitch").unwrap_or(80.0) * s];
        }
        let walk_speed = walk.travel.length() / walk.anim.duration.max(1e-3);
        let turn_radius = std::fs::read_to_string(self.tune_dir.join(name).join("mcAmbientType.xml")).ok().and_then(|t| xml_value(&t, "fMinTurningRadius")).unwrap_or(0.1);
        Some(PedType { female, voice: core, model, walk, idle, evade, angry, pivot, lod_switch, walk_speed, turn_radius })
    }

    /// The files of the types the spawner can draw around `focus` now, read and parsed on worker threads (two at a
    /// time) before they are first wanted, so building one does not stop the game - docs/PERFORMANCE.md 2026-10-23 (5).
    fn read_ahead(&mut self, focus: Vec3, hour: f32) {
        if self.reading.values().filter(|h| !h.is_finished()).count() >= 2 {
            return;
        }
        let names: Vec<String> = self.districts.names(focus, hour).into_iter().filter(|n| n.starts_with("ped_")).map(|n| n.to_string()).collect();
        for n in names {
            let Some(ti) = self.dirs.iter().position(|d| d.0.eq_ignore_ascii_case(&n)) else { continue };
            if self.types.contains_key(&ti) || self.reading.contains_key(&ti) {
                continue;
            }
            let (m, a) = (self.dirs[ti].1.clone(), self.dirs[ti].2.clone());
            if let Ok(h) = std::thread::Builder::new().name("ped read".into()).spawn(move || read_type(&m, &a)) {
                self.reading.insert(ti, h);
            }
            if self.reading.values().filter(|h| !h.is_finished()).count() >= 2 {
                break;
            }
        }
    }

    fn load_type(&mut self, gpu: &mut Gpu, ti: usize) -> bool {
        if !self.types.contains_key(&ti) {
            let t = self.build_type(gpu, ti);
            if t.is_none() {
                log::warn!("peds: could not load {}", self.dirs[ti].0);
            }
            self.types.insert(ti, t);
        }
        self.types.get(&ti).map(|t| t.is_some()).unwrap_or(false)
    }

    /// Distance from the lane centre to the middle of the pavement on its right, if there
    /// is a kerb within reach (only the outermost lane has one).
    fn curb_offset(&mut self, nav: &NavSys, col: &WorldCollider, li: usize) -> Option<f32> {
        if let Some(c) = self.curb.get(&li) {
            return *c;
        }
        let l = &nav.links[li];
        if l.len < 6.0 {
            return None;
        }
        let right = l.dir().cross(Vec3::Y).normalize_or_zero();
        // only the outermost lane borders the pavement
        let mid = (l.a + l.b) * 0.5;
        if l.side.iter().any(|&n| ((nav.links[n].a + nav.links[n].b) * 0.5 - mid).dot(right) > 1.0) {
            self.curb.insert(li, None);
            return None;
        }
        let mut found: Option<f32> = None;
        let mut votes = 0;
        // the kerb has to be there at several points along the link
        for t in [0.25f32, 0.5, 0.75] {
            let p = l.a.lerp(l.b, t);
            // collision not streamed in yet: decide later
            let h0 = col.ground(p, 1.0, 2.5)?;
            let mut prev = h0.point.y;
            let mut off = 1.0;
            while off <= 11.0 {
                let q = p + right * off;
                let Some(h) = col.ground(Vec3::new(q.x, prev, q.z), 0.8, 1.2) else { break };
                let step = h.point.y - prev;
                if (0.06..0.4).contains(&step) {
                    // room to walk behind the kerb?
                    let q2 = p + right * (off + 1.4);
                    if let Some(h2) = col.ground(Vec3::new(q2.x, h.point.y, q2.z), 0.5, 0.5) {
                        if (h2.point.y - h.point.y).abs() < 0.15 && h2.normal.y > 0.95 {
                            votes += 1;
                            found = Some(found.map(|f: f32| f.max(off + 1.2)).unwrap_or(off + 1.2));
                        }
                    }
                    break;
                }
                if step.abs() > 0.4 {
                    break;
                }
                prev = h.point.y;
                off += 0.4;
            }
        }
        if std::env::var("MCLA_PED_KERB").is_ok() && self.curb.len() < 40 {
            let p = l.a.lerp(l.b, 0.5);
            let prof: Vec<String> = (0..24).map(|k| col.ground(p + right * (k as f32 * 0.5) + Vec3::Y * 0.5, 1.0, 3.0).map(|h| format!("{:.2}", h.point.y - p.y)).unwrap_or("-".into())).collect();
            log::info!("kerb link {} len {:.0} sides {} votes {} found {:?}: {}", li, l.len, l.side.len(), votes, found, prof.join(" "));
        }
        let r = if votes >= 2 { found } else { None };
        self.curb.insert(li, r);
        r
    }

    /// A pedestrian set for this place and hour, from the district's distribution.
    fn pick_type(&self, pos: Vec3, h: u32) -> Option<usize> {
        if !self.districts.loaded() {
            // no district data at all: anything goes
            return Some(h as usize % self.dirs.len());
        }
        let r = (h & 0xffffff) as f32 / 16777216.0;
        let name = self.districts.pick(pos, self.hour, r, &|n| n.starts_with("ped_"))?;
        self.dirs.iter().position(|d| d.0.eq_ignore_ascii_case(name))
    }

    pub fn clear(&mut self) {
        self.list.clear();
        self.active.clear();
        self.filled = false;
    }

    /// Positions and headings of the pedestrians closest to `p` (diagnostics).
    pub fn nearest(&self, p: Vec3, n: usize) -> Vec<(Vec3, Vec3, String)> {
        let mut v: Vec<&Ped> = self.list.iter().collect();
        v.sort_by(|a, b| a.pos.distance_squared(p).partial_cmp(&b.pos.distance_squared(p)).unwrap());
        v.into_iter().take(n).map(|q| (q.pos, q.rot * -Vec3::Z, self.dirs[q.ty].0.clone())).collect()
    }

    /// (links with a pavement, links without) among those examined so far.
    pub fn curb_stats(&self) -> (usize, usize) {
        let yes = self.curb.values().filter(|c| c.is_some()).count();
        (yes, self.curb.len() - yes)
    }

    pub fn count(&self) -> usize {
        self.list.len()
    }

    /// Walkers standing at a kerb waiting to cross.
    pub fn waiting(&self) -> usize {
        self.list.iter().filter(|p| p.plan.is_some()).count()
    }

    /// Within the view cone near the player (sub_826F0628, pedestrian branch): closer than
    /// the inner spawn radius and at an angle to the view line (either way along it) below
    /// the cone's half angle, which narrows with speed.
    fn in_view(&self, v: Vec3) -> bool {
        if flat(v).length() > self.tune.spawn_inner {
            return false;
        }
        let t = ((self.speed_mph - self.tune.fov_mph[0]) / (self.tune.fov_mph[1] - self.tune.fov_mph[0])).clamp(0.0, 1.0);
        let fov = (self.tune.fov_deg[0] + (self.tune.fov_deg[1] - self.tune.fov_deg[0]) * t).to_radians();
        let c = self.view_dir.dot(v.normalize_or_zero()).abs().min(1.0);
        c.acos() < fov
    }

    /// Outside the band the pedestrians live in (sub_826F49B8): more than half the box width
    /// to the side of the view line, or further than the unspawn radius ahead (half of it
    /// behind).
    fn out_of_range(&self, focus: Vec3, pos: Vec3) -> bool {
        let v = pos - focus;
        let dir = self.view_dir;
        let lateral = (v.z * dir.x - v.x * dir.z).abs();
        let lim = if v.dot(dir) < 0.0 { self.tune.unspawn * 0.5 } else { self.tune.unspawn };
        lateral > self.tune.box_width * 0.5 || flat(v).length() > lim
    }

    /// One run of the original's pedestrian spawner (sub_826F3038) and unspawner
    /// (sub_826F49B8, from sub_826F6130) on the navigation mesh: every other 30 Hz frame.
    fn spawn_frame(&mut self, gpu: &mut Gpu, mesh: &NavMesh, col: &WorldCollider, focus: Vec3) {
        // unspawn: 30 random checks a frame
        for _ in 0..30 {
            if self.list.is_empty() {
                break;
            }
            let i = (self.rand() * self.list.len() as f32) as usize % self.list.len();
            // (an item's person goes like any walker, sub_826F49B8: no item test)
            if self.out_of_range(focus, self.list[i].pos) {
                let p = self.list.swap_remove(i);
                if std::env::var("MCLA_PED_DUMP").is_ok() {
                    let v = p.pos - focus;
                    log::info!("ped unspawn {} dist {:.1} ahead {:.1} lateral {:.1}", p.key.1, flat(v).length(), v.dot(self.view_dir), (v.z * self.view_dir.x - v.x * self.view_dir.z).abs());
                }
            }
        }
        // candidates: the sidewalk polygons touching the sphere of the outer spawn radius,
        // the first 128 (sub_826F05C8)
        self.cands = spawn_candidates(mesh, focus, self.tune.spawn_outer);
        let total: f32 = self.cands.iter().map(|c| c.1).sum();
        // area times the density, times the focus district's density at this hour
        self.target = total * self.tune.density * self.districts.density_at(focus, self.hour);
        if self.list.len() as f32 >= self.target || self.cands.is_empty() {
            return;
        }
        // one type per frame, from the focus district's list
        let r0 = self.rand();
        let Some(ti) = self.pick_type(focus, (r0 * 16777216.0) as u32) else { return };
        if !self.load_type(gpu, ti) {
            return;
        }
        // polygons ahead weigh ten times their area
        let dir = self.view_dir;
        let weights: Vec<f32> = self.cands.iter().map(|&(i, a)| if (mesh.polys[i as usize].center - focus).dot(dir) > 0.0 { a * 10.0 } else { a }).collect();
        let wsum: f32 = weights.iter().sum();
        let mut spawned = 0;
        let mut occ: Option<Option<crate::occluder::Selected>> = None;
        for _ in 0..60 {
            if spawned >= 3 || self.list.len() as f32 >= self.target {
                break;
            }
            let mut x = self.rand() * wsum;
            let mut k = weights.len() - 1;
            for (j, w) in weights.iter().enumerate() {
                x -= w;
                if x < 0.0 {
                    k = j;
                    break;
                }
            }
            let pi = self.cands[k].0;
            let q = &mesh.polys[pi as usize];
            let (r1, r2) = (self.rand(), self.rand());
            let mut pos = random_point(q, r1, r2);
            let v = pos - focus;
            if (v.z * dir.x - v.x * dir.z).abs() > self.tune.box_width * 0.5 {
                continue;
            }
            // in view, unless an occluder hides it from just above the player (no test in the
            // front-end mode)
            if !self.frontend && self.in_view(v) {
                if occ.is_none() {
                    occ = Some(crate::occluder::city(&self.data_dir).map(|c| c.select_around(focus + Vec3::Y * OCC_EYE, crate::occluder::RANGE * OCC_RANGE_SCALE)));
                }
                if !occ.as_ref().and_then(|o| o.as_ref()).map_or(false, |o| o.hides_sphere(pos + Vec3::Y * OCC_LIFT, OCC_RADIUS)) {
                    continue;
                }
            }
            let (s1, s2) = (self.tune.ped_spacing, self.tune.type_spacing);
            if self.list.iter().any(|p| {
                let d = p.pos.distance_squared(pos);
                d < s1 * s1 || (p.ty == ti && d < s2 * s2)
            }) {
                continue;
            }
            // the collision's floor where it agrees with the mesh
            if let Some(g) = col.ground(pos, 1.0, 1.5) {
                if (g.point.y - pos.y).abs() < 0.5 {
                    pos.y = g.point.y;
                }
            }
            let a = self.rand() * std::f32::consts::TAU;
            let face = Vec3::new(a.sin(), 0.0, a.cos());
            // the walking speed factor, drawn once (sub_82797CC8)
            let scale = 0.85 + 0.3 * self.rand();
            let id = self.next_id;
            self.next_id = self.next_id.wrapping_add(1);
            let seed = hash(id, pi);
            if std::env::var("MCLA_PED_DUMP").is_ok() {
                log::info!("ped spawn {} dist {:.1} ahead {:.1} lateral {:.1} target {:.2}", id, flat(v).length(), v.dot(dir), (v.z * dir.x - v.x * dir.z).abs(), self.target);
            }
            self.list.push(Ped {
                ty: ti,
                key: (usize::MAX, id),
                pos,
                rot: Quat::from_rotation_y((-face.x).atan2(-face.z)),
                link: 0,
                fwd: true,
                lat: 0.0,
                state: State::Idle,
                time: ((seed >> 8) & 0xff) as f32 / 40.0,
                from: Vec::new(),
                blend: 0.0,
                cur: Vec::new(),
                loiter: false,
                calm: 0.0,
                seed,
                plan: None,
                cross: None,
                wait: 0.0,
                poly: pi,
                travel: face,
                path: Vec::new(),
                since_plan: 0,
                scale,
                speed: 0.0,
                rail: None,
                stop: 0.0,
                stops: 0,
                slide: None,
                turn_round: false,
                turn_rest: 0.0,
                then_angry: false,
                dodged: None,
                hold: 0.5,
                item: None,
                age: 0.0,
                life: 8.0,
                blend_len: 0.25,
            });
            spawned += 1;
        }
    }

    /// The frame's one path request (sub_82797B20 lets one through per frame, to a walker
    /// without a path - it keeps one until it is cancelled - whose last request is at least
    /// 16 frames old): a random walk from where it stands, the way it faces (backwards after
    /// an even number of stops in a row, more than one), with the points on the edges `frac`
    /// (0.3 .. 0.7, per path) along them (sub_827979A8).
    fn plan_frame(&mut self, mesh: &NavMesh) {
        for p in self.list.iter_mut() {
            p.since_plan = p.since_plan.saturating_add(1);
        }
        let n = self.list.len();
        for k in 0..n {
            let i = (self.plan_cursor + k) % n;
            let p = &self.list[i];
            if p.poly == NO_POLY || p.hold > 0.0 || p.since_plan < 16 || !p.path.is_empty() || !matches!(p.state, State::Walk | State::Idle) {
                continue;
            }
            let start = if mesh.polys[p.poly as usize].contains(p.pos.x, p.pos.z) { p.poly } else { mesh.find(p.pos, AI_SIDEWALK, 2.0).unwrap_or(p.poly) };
            let rev = p.stops > 1 && p.stops % 2 == 0;
            let (pos, heading) = (p.pos, p.rot * -Vec3::Z * if rev { -1.0 } else { 1.0 });
            let mut rng = self.rng;
            let mut rand = || {
                rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
                (rng >> 8) as f32 / 16777216.0
            };
            let hops = random_path(mesh, start, pos, heading, &mut rand);
            let frac = 0.3 + 0.4 * rand();
            self.rng = rng;
            let p = &mut self.list[i];
            p.poly = start;
            p.path = hops.into_iter().map(|(a, b, q)| (a + (b - a) * frac, q)).collect();
            p.since_plan = 0;
            p.rail = None;
            p.turn_round = rev && !p.path.is_empty();
            self.plan_cursor = (i + 1) % n;
            break;
        }
    }

    /// MCLA_PED_TRACE=file: the focus (F) and every pedestrian (P) ten times a second, as
    /// the recordings of the original are written (work/people2/re/peds_rec2.py).
    fn write_trace(&mut self, focus: Vec3, dt: f32) {
        use std::io::Write;
        let Some((w, t, next)) = self.trace.as_mut() else { return };
        *t += dt;
        if *t < *next {
            return;
        }
        *next = *t + 0.1;
        let _ = writeln!(w, "{:.2}	F	{:.2}	{:.2}	{:.2}	{:.3}	{:.3}	{:.2}	{}	{}	{:.2}	{:?}", t, focus.x, focus.y, focus.z, self.view_dir.x, self.view_dir.z, self.target, self.list.len(), self.cands.len(), self.hour, self.districts.district(focus));
        for p in &self.list {
            let st = match p.state {
                State::Walk => "walk",
                State::Idle => "idle",
                State::Evade(_) => "evade",
                State::Angry(_) => "angry",
                State::Turn(_) => "turn",
            };
            let _ = writeln!(w, "{:.2}	P	{}	{:.2}	{:.2}	{:.2}	{}	{}	{:.2}	{}	{}	{}	{}", t, p.key.1, p.pos.x, p.pos.y, p.pos.z, (p.state == State::Walk) as u8, st, p.stop, p.stops, p.path.len(), p.rail.map_or(0, |r| 1 + r.1 as u8), p.slide.is_some() as u8);
        }
        let _ = w.flush();
    }

    fn populate(&mut self, gpu: &mut Gpu, nav: &NavSys, col: &WorldCollider, center: Vec3) {
        let unspawn = self.tune.unspawn;
        let active = &mut self.active;
        self.list.retain(|p| {
            let keep = p.pos.distance_squared(center) < unspawn * unspawn;
            if !keep {
                active.remove(&p.key);
            }
            keep
        });
        // the first time everything around is filled in, afterwards only the far ring
        let inner = if self.filled { self.tune.spawn_inner } else { 5.0 };
        let outer = self.tune.spawn_outer;
        let mut any_ground = false;
        let links: Vec<usize> = nav.links_near(center, outer).collect();
        for li in links {
            let l = &nav.links[li];
            let mid = (l.a + l.b) * 0.5;
            if mid.distance(center) > outer + l.len * 0.5 || (mid.y - center.y).abs() > 25.0 {
                continue;
            }
            let want = l.len * self.tune.density * self.districts.density.at(self.hour);
            let n = want.floor() as u32 + if ((hash(li as u32, 77) & 0xffff) as f32 / 65536.0) < want.fract() { 1 } else { 0 };
            if n == 0 || col.ground(mid, 1.0, 2.5).is_none() {
                continue;
            }
            any_ground = true;
            let Some(lat) = self.curb_offset(nav, col, li) else { continue };
            let right = l.dir().cross(Vec3::Y).normalize_or_zero();
            for s in 0..n {
                let key = (li, s);
                if self.active.contains(&key) {
                    continue;
                }
                let h = hash(li as u32, s + 1);
                let t = 0.1 + 0.8 * (h & 0xffff) as f32 / 65536.0;
                let jitter = (((h >> 16) & 0xff) as f32 / 255.0 - 0.5) * 0.8;
                let pos = l.a.lerp(l.b, t) + right * (lat + jitter);
                let d = pos.distance(center);
                if d < inner || d > outer {
                    continue;
                }
                let Some(g) = col.ground(pos, 1.0, 1.5) else { continue };
                // a type that is not already walking nearby
                let sp = self.tune.type_spacing;
                let mut ti = usize::MAX;
                for attempt in 0..4 {
                    let Some(c) = self.pick_type(pos, hash(h, 9 + attempt)) else { break };
                    ti = c;
                    if !self.list.iter().any(|p| p.ty == c && p.pos.distance_squared(pos) < sp * sp) {
                        break;
                    }
                }
                if ti == usize::MAX {
                    continue;
                }
                if !self.load_type(gpu, ti) {
                    continue;
                }
                let fwd = h & 0x0100_0000 != 0;
                let loiter = (h >> 25) % 5 == 0;
                let face = if loiter { -right } else if fwd { l.dir() } else { -l.dir() };
                self.active.insert(key);
                self.list.push(Ped {
                    ty: ti,
                    key,
                    pos: g.point,
                    rot: Quat::from_rotation_y((-face.x).atan2(-face.z)),
                    link: li,
                    fwd,
                    lat: lat + jitter,
                    state: if loiter { State::Idle } else { State::Walk },
                    time: ((h >> 8) & 0xff) as f32 / 40.0,
                    from: Vec::new(),
                    blend: 0.0,
                    cur: Vec::new(),
                    loiter,
                    calm: 0.0,
                    seed: h,
                    plan: None,
                    cross: None,
                    wait: 0.0,
                    poly: NO_POLY,
                    travel: face,
                    path: Vec::new(),
                    since_plan: 0,
                    scale: 1.0,
                    speed: 0.0,
                    rail: None,
                    stop: 0.0,
                    stops: 0,
                    slide: None,
                    turn_round: false,
                    turn_rest: 0.0,
                    then_angry: false,
                    dodged: None,
                    hold: 0.0,
                    item: None,
                    age: 0.0,
                    life: 8.0,
                    blend_len: 0.25,
                });
            }
        }
        if any_ground {
            self.filled = true;
        }
    }

    pub fn update(&mut self, gpu: &mut Gpu, nav: &NavSys, col: &WorldCollider, obstacles: &dyn Fn(Vec3, f32, &mut Vec<Obst>), center: Vec3, threats: &[Threat], signals: Option<(&crate::signals::Signals, f32)>, hour: f32, dt: f32) {
        self.hour = hour;
        if !self.enabled || self.dirs.is_empty() {
            return;
        }
        let mut frame0 = self.frame_no;
        if let Some(mesh) = self.mesh.take() {
            // the original's 30 Hz frames
            self.step = (self.step + dt).min(FRAME * 4.0);
            frame0 = self.frame_no;
            if self.districts.loaded() && self.frame_no % 8 == 0 {
                self.read_ahead(center, self.hour);
                if let Some((p, h)) = self.ahead_also {
                    self.read_ahead(p, h);
                }
            }
            while self.step >= FRAME || self.preroll > 0 {
                if self.preroll > 0 {
                    self.preroll -= 1;
                } else {
                    self.step -= FRAME;
                }
                self.frame_no = self.frame_no.wrapping_add(1);
                if self.frame_no % 2 == 0 {
                    self.spawn_frame(gpu, &mesh, col, center);
                }
                self.plan_frame(&mesh);
            }
            self.mesh = Some(mesh);
            self.write_trace(center, dt);
        } else {
            self.timer -= dt;
            if self.timer <= 0.0 {
                self.timer = 0.4;
                self.populate(gpu, nav, col, center);
            }
        }
        let mut rng = self.rng;
        let mut rand = move || {
            rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
            (rng >> 8) as f32 / 16777216.0
        };
        let mut near: Vec<(Vec3, f32)> = Vec::new();
        let mut boxes: Vec<Obst> = Vec::new();
        let peds_pos: Vec<Vec3> = self.list.iter().map(|p| p.pos).collect();
        let has_mesh = self.mesh.is_some();
        for i in 0..self.list.len() {
            let Some(Some(ty)) = self.types.get(&self.list[i].ty) else { continue };
            let p = &mut self.list[i];
            if p.item.is_some() {
                // (an item's person, sub_826DFB08: flag 0x08 clear - no walking, no avoiding;
                // no rail - locomotion state 0, the set's clip 0 (`*Ped_Idle*`) looped from 0
                // at rate 1, sub_8279EF40 / sub_826ECF58. The car check sub_8279B538 runs all
                // the same: a dodge, then clip 0 again with its 1 s blend, sub_826EC8F0)
                p.age += dt;
                p.blend = (p.blend - dt).max(0.0);
                if p.state == State::Idle && !ty.evade.is_empty() {
                    if let Some((car, away, speed)) = car_threat(p.pos, threats) {
                        let fwd = flat(p.rot * -Vec3::Z).normalize_or(Vec3::Z);
                        let from_front = fwd.dot(flat(car - p.pos).normalize_or_zero()) > 0.4;
                        let mut best = (f32::MIN, 0usize);
                        for (k, c) in ty.evade.iter().enumerate() {
                            if c.front && !from_front {
                                continue;
                            }
                            let s = (p.rot * c.travel.normalize_or_zero()).dot(away) + rand() * 0.5;
                            if s > best.0 {
                                best = (s, k);
                            }
                        }
                        p.from = p.cur.clone();
                        p.blend = 0.15;
                        p.blend_len = 0.25;
                        p.state = State::Evade(best.1);
                        p.time = 0.0;
                        p.dodged = Some(car);
                        self.evades += 1;
                        self.speech.push(Speech { pos: p.pos, voice: ty.voice.clone(), female: ty.female, fast: ty.evade[best.1].fast || speed * speed > 244.77 });
                    }
                }
                let t0 = p.time;
                p.time += dt;
                if let State::Evade(k) = p.state {
                    let clip = &ty.evade[k];
                    let (a, ar) = clip.mover(clip.frame(t0, false));
                    let (b, br) = clip.mover(clip.frame(p.time, false));
                    p.pos += p.rot * (ar.inverse() * (b - a));
                    p.rot = (p.rot * (ar.inverse() * br)).normalize();
                    if p.time >= clip.anim.duration.max(0.05) {
                        p.from = p.cur.clone();
                        p.blend = 1.0;
                        p.blend_len = 1.0;
                        p.time = 0.0;
                        p.state = State::Idle;
                    }
                    let f = p.rot * -Vec3::Z;
                    p.rot = Quat::from_rotation_y((-f.x).atan2(-f.z));
                    if let Some(g) = col.ground(p.pos, 0.7, 1.5) {
                        p.pos.y += (g.point.y - p.pos.y) * (dt * 12.0).min(1.0);
                    }
                }
                continue;
            }
            p.calm = (p.calm - dt).max(0.0);
            p.blend = (p.blend - dt).max(0.0);
            let on_mesh = has_mesh && p.poly != NO_POLY;
            // on the mesh: the original's sidewalk behaviour
            let mut rate = 1.0;
            if on_mesh {
                boxes.clear();
                obstacles(p.pos, 4.0, &mut boxes);
                // the spheres due in the 30 Hz frames run now
                let d = p.pos.distance(center);
                let chance = AWARE_CHANCE.iter().find(|c| d < c.0).map_or(AWARE_CHANCE_FAR, |c| c.1);
                let mut due = 0u8;
                let mut n = frame0;
                while n != self.frame_no {
                    n = n.wrapping_add(1);
                    if rand() < chance {
                        due |= 1 << (n % 3);
                    }
                }
                rate = sidewalk_step(p, ty, i, &peds_pos, &boxes, Some(col), threats, due, dt, &mut rand, &mut self.evades, &mut self.speech);
            }
            // a car about to pass too close?
            if !on_mesh && matches!(p.state, State::Walk | State::Idle) && p.calm <= 0.0 && !ty.evade.is_empty() {
                for t in threats {
                    let speed = t.vel.length();
                    if speed < 3.0 {
                        continue;
                    }
                    let v = t.vel / speed;
                    let rel = p.pos - t.pos;
                    let along = rel.dot(v);
                    if rel.y.abs() > 4.0 || along < -1.0 || along > speed * 1.2 + 4.0 {
                        continue;
                    }
                    let side = rel - v * along;
                    if side.length() > self.evade_radius {
                        continue;
                    }
                    let away = if side.length() > 0.2 { side.normalize() } else { v.cross(Vec3::Y) };
                    let from_front = (p.rot * -Vec3::Z).dot(v) < -0.4;
                    // of the game's evade clips, one that actually leads out of the car's way
                    let mut best = (f32::MIN, 0usize);
                    for (k, c) in ty.evade.iter().enumerate() {
                        if c.front && !from_front {
                            continue;
                        }
                        let s = (p.rot * c.travel.normalize_or_zero()).dot(away) + rand() * 0.5;
                        if s > best.0 {
                            best = (s, k);
                        }
                    }
                    p.from = p.cur.clone();
                    p.blend = 0.15;
                    p.state = State::Evade(best.1);
                    p.time = 0.0;
                    self.evades += 1;
                    self.speech.push(Speech { pos: p.pos, voice: ty.voice.clone(), female: ty.female, fast: ty.evade[best.1].fast || speed > 18.0 });
                    break;
                }
            }
            let (clip, looped) = ty.clip(p.state);
            // advance, carrying the animation's own root motion into the world (walkers on the
            // mesh move along their rails instead; the walk clip plays at their speed)
            let t0 = p.time;
            p.time += dt * rate;
            if on_mesh && looped {
                // root motion off
            } else {
            let last = clip.last_frame();
            let apply = |p: &mut Ped, f0: f32, f1: f32| {
                let (a, ar) = clip.mover(f0);
                let (b, br) = clip.mover(f1);
                p.pos += p.rot * (ar.inverse() * (b - a));
                p.rot = (p.rot * (ar.inverse() * br)).normalize();
            };
            if looped {
                let (f0, f1) = (clip.frame(t0, true), clip.frame(p.time, true));
                if f1 >= f0 {
                    apply(p, f0, f1);
                } else {
                    apply(p, f0, last);
                    apply(p, 0.0, f1);
                }
            } else {
                apply(p, clip.frame(t0, false), clip.frame(p.time, false));
                if let State::Turn(_) = p.state {
                    // what the clip does not turn, at most pi/4 per 8 frames
                    let a = p.turn_rest.clamp(-TURN_RATE * dt, TURN_RATE * dt);
                    p.rot = (p.rot * Quat::from_rotation_y(a)).normalize();
                    p.turn_rest -= a;
                }
                // a Pivot clip ends 8 frames early (blended into the next)
                let end = if matches!(p.state, State::Turn(_)) { (clip.anim.duration - BLEND_S).max(0.05) } else { clip.anim.duration.max(0.05) };
                if p.time >= end {
                    p.from = p.cur.clone();
                    p.blend = 0.25;
                    p.time = 0.0;
                    p.calm = 1.5;
                    let angry = |r: f32| State::Angry((r * ty.angry.len() as f32) as usize % ty.angry.len());
                    p.state = if on_mesh {
                        // after a dodge: face the car (a Pivot clip beyond 45 degrees), then
                        // the angry clip, else wait (sub_8279B468)
                        let f = flat(p.rot * -Vec3::Z).normalize_or(Vec3::Z);
                        match p.state {
                            State::Evade(_) => {
                                let want = p.dodged.map_or(0.0, |c| yaw_between(f, flat(c - p.pos).normalize_or(f)));
                                if want.abs() > std::f32::consts::FRAC_PI_4 && !ty.pivot.is_empty() {
                                    let k = (0..ty.pivot.len()).min_by(|&a, &b| wrap_angle(want - ty.pivot[a].1).abs().total_cmp(&wrap_angle(want - ty.pivot[b].1).abs())).unwrap();
                                    p.turn_rest = wrap_angle(want - ty.pivot[k].1);
                                    p.then_angry = true;
                                    State::Turn(k)
                                } else if !ty.angry.is_empty() {
                                    angry(rand())
                                } else {
                                    p.stop = stop_time(ty.idle.anim.duration, &mut rand);
                                    p.stops += 1;
                                    State::Idle
                                }
                            }
                            State::Turn(_) if p.then_angry => {
                                p.then_angry = false;
                                if !ty.angry.is_empty() {
                                    angry(rand())
                                } else {
                                    p.stop = stop_time(ty.idle.anim.duration, &mut rand);
                                    p.stops += 1;
                                    State::Idle
                                }
                            }
                            _ => State::Idle,
                        }
                    } else {
                        match p.state {
                            State::Evade(_) if !ty.angry.is_empty() && rand() < 0.45 => angry(rand()),
                            _ if p.loiter => State::Idle,
                            _ => State::Walk,
                        }
                    };
                }
            }
            }
            // stay upright; walkers steer along the pavement
            let f = p.rot * -Vec3::Z;
            let mut yaw = (-f.x).atan2(-f.z);
            let mesh = self.mesh.as_ref().filter(|_| p.poly != NO_POLY);
            if mesh.is_none() && p.state == State::Walk && p.cross.is_none() && p.plan.is_none() {
                let l = &nav.links[p.link];
                let s = (p.pos - l.a).dot(l.dir());
                // end of the block: carry on along the next link if it has a pavement, cross
                // over to the other side of the street, or turn back
                if (p.fwd && s > l.len - 1.0) || (!p.fwd && s < 1.0) {
                    let cands: &Vec<usize> = if p.fwd { &l.next } else { &l.prev };
                    let mut go = None;
                    for &n in cands {
                        if nav.links[n].dir().dot(l.dir()) > 0.6 {
                            if let Some(Some(lat)) = self.curb.get(&n) {
                                go = Some((n, *lat));
                            }
                        }
                    }
                    let mut over: Option<(f32, Vec3, usize, f32)> = None;
                    if go.is_none() || rand() < 0.35 {
                        for n in nav.links_near(p.pos, 34.0) {
                            let l2 = &nav.links[n];
                            if l2.dir().dot(l.dir()) > -0.8 {
                                continue;
                            }
                            let Some(Some(lat2)) = self.curb.get(&n) else { continue };
                            let s2 = (p.pos - l2.a).dot(l2.dir()).clamp(1.0, (l2.len - 1.0).max(1.0));
                            let q = l2.a + l2.dir() * s2 + l2.dir().cross(Vec3::Y).normalize_or_zero() * *lat2;
                            let d = q.distance(p.pos);
                            if d < 7.0 || d > 34.0 || (q - p.pos).dot(l.dir()).abs() > d * 0.45 || (q.y - p.pos.y).abs() > 2.0 {
                                continue;
                            }
                            if over.map_or(true, |b| d < b.0) {
                                over = Some((d, q, n, *lat2));
                            }
                        }
                    }
                    match (over, go) {
                        (Some((_, q, n, lat)), _) => p.plan = Some(Plan::Cross(q, n, lat)),
                        (None, Some((n, lat))) => p.plan = Some(Plan::Straight(n, lat + ((p.seed >> 16) & 0xff) as f32 / 255.0 * 0.8 - 0.4)),
                        (None, None) => p.fwd = !p.fwd,
                    }
                    p.wait = 0.0;
                }
            }
            // at the kerb: go when the lights (or, without lights, the traffic) allow it
            if let (Some(plan), true) = (p.plan, matches!(p.state, State::Walk | State::Idle)) {
                let travel = if mesh.is_some() {
                    p.travel
                } else {
                    let l = &nav.links[p.link];
                    if p.fwd { l.dir() } else { -l.dir() }
                };
                // signal for the traffic of the own street at the junction ahead
                let sig = signals.and_then(|(sg, t)| sg.ahead(p.pos, travel, t)).filter(|a| a.1 < 30.0).map(|a| a.2);
                let (line, reach) = match plan {
                    Plan::Straight(..) => (travel, 26.0),
                    Plan::Cross(q, ..) => ((q - p.pos).normalize_or_zero(), q.distance(p.pos) + 4.0),
                };
                // a car driving at the stretch about to be walked?
                let mut busy = false;
                for t in threats {
                    let speed = t.vel.length();
                    if speed < 2.5 {
                        continue;
                    }
                    let rel = t.pos - p.pos;
                    let along = rel.dot(line);
                    if rel.y.abs() > 4.0 || along < -3.0 || along > reach {
                        continue;
                    }
                    let side = rel - line * along;
                    // closing in on the crossing line within the next few seconds
                    let closing = -side.normalize_or_zero().dot(t.vel);
                    if side.length() < 3.0 || (closing > 1.0 && side.length() / closing < if sig.is_some() { 1.5 } else { 4.5 }) {
                        busy = true;
                        break;
                    }
                }
                let lights = match (plan, sig) {
                    (_, None) => true,
                    (Plan::Straight(..), Some(st)) => st == 2,
                    (Plan::Cross(..), Some(st)) => st == 0,
                };
                if lights && !busy {
                    match plan {
                        Plan::Straight(n, lat) => {
                            p.link = n;
                            p.lat = lat;
                        }
                        Plan::Cross(q, n, lat) => p.cross = Some((q, n, lat)),
                    }
                    p.plan = None;
                    if p.state == State::Idle {
                        p.from = p.cur.clone();
                        p.blend = 0.25;
                        p.time = 0.0;
                        p.state = State::Walk;
                    }
                } else {
                    p.wait += dt;
                    if p.state == State::Walk {
                        p.from = p.cur.clone();
                        p.blend = 0.25;
                        p.time = 0.0;
                        p.state = State::Idle;
                    }
                    // tired of waiting where there are no lights: walk back
                    // (on the mesh also at lights that never come round for this line)
                    if p.wait > 25.0 && (sig.is_none() || (mesh.is_some() && p.wait > 45.0)) {
                        p.plan = None;
                        p.fwd = !p.fwd;
                        p.from = p.cur.clone();
                        p.blend = 0.25;
                        p.time = 0.0;
                        p.state = State::Walk;
                    }
                }
            }
            if p.state == State::Walk && mesh.is_none() {
                if let Some((q, n, lat)) = p.cross {
                    if Vec3::new(q.x - p.pos.x, 0.0, q.z - p.pos.z).length() < 0.8 {
                        // over: walk away from the corner on the far side
                        p.link = n;
                        p.lat = lat;
                        p.cross = None;
                        self.crossings += 1;
                    }
                }
                let l = &nav.links[p.link];
                let dir = l.dir();
                let right = dir.cross(Vec3::Y).normalize_or_zero();
                let s = (p.pos - l.a).dot(dir);
                let mut target = l.a + dir * (s + if p.fwd { 2.5 } else { -2.5 }) + right * p.lat;
                if let Some((q, ..)) = p.cross {
                    target = q;
                }
                // walk around benches, lamp posts, news stands
                boxes.clear();
                obstacles(p.pos, 2.6, &mut boxes);
                near.clear();
                near.extend(boxes.iter().map(|o| (o.0, o.2.length().min(o.2.max_element() * 1.1))));
                let side = Vec3::new(-f.z, 0.0, f.x);
                for (o, r) in &near {
                    let rel = Vec3::new(o.x - p.pos.x, 0.0, o.z - p.pos.z);
                    let ahead = rel.dot(f);
                    let lateral = rel.dot(side);
                    let clear = r + 0.45;
                    if ahead > -0.3 && ahead < 2.6 + r && lateral.abs() < clear {
                        // pass on the side we are already on (towards the buildings when dead ahead)
                        let pass = if lateral.abs() < 0.08 { -right.dot(side).signum() } else { lateral.signum() };
                        target -= side * pass * (clear - lateral.abs()) * 2.5;
                    }
                    let d = rel.length();
                    if d < clear * 0.8 && d > 1e-3 {
                        p.pos -= rel / d * (clear * 0.8 - d).min(dt * 2.0);
                    }
                }
                let to = target - p.pos;
                let d = wrap_angle((-to.x).atan2(-to.z) - yaw);
                yaw += d.clamp(-1.8 * dt, 1.8 * dt);
            }
            p.rot = Quat::from_rotation_y(yaw);
            // on the mesh: the polygon's height, refined by the collision where the two agree
            let floor = mesh.and_then(|m| {
                if !m.polys[p.poly as usize].contains(p.pos.x, p.pos.z) {
                    if let Some(k) = m.find(p.pos, !0, 2.0) {
                        p.poly = k;
                    }
                }
                let q = &m.polys[p.poly as usize];
                q.contains(p.pos.x, p.pos.z).then(|| q.height(p.pos.x, p.pos.z))
            });
            let y = match (col.ground(p.pos, 0.7, 1.5), floor) {
                (Some(g), Some(m)) => Some(if (g.point.y - m).abs() < 0.5 { g.point.y } else { m }),
                (Some(g), None) => Some(g.point.y),
                (None, m) => m,
            };
            if let Some(y) = y {
                p.pos.y += (y - p.pos.y) * (dt * 12.0).min(1.0);
            }
        }
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223 + self.list.len() as u32);
    }

    /// sub_826DFB08: a layout item's person, standing at the item's place facing
    /// (-sin h, 0, -cos h). False when the type is not one of the people's.
    pub fn spawn_item(&mut self, gpu: &mut Gpu, col: &WorldCollider, item: u32, name: &str, pos: Vec3, heading: f32) -> bool {
        let Some(ti) = self.dirs.iter().position(|d| d.0.eq_ignore_ascii_case(name)) else { return false };
        if !self.load_type(gpu, ti) {
            return false;
        }
        let mut pos = pos;
        if let Some(g) = col.ground(pos, 1.0, 1.5) {
            pos.y = g.point.y;
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let life = 8.0 + 4.0 * self.rand();
        self.list.push(Ped {
            ty: ti,
            key: (usize::MAX, id),
            pos,
            rot: Quat::from_rotation_y(heading),
            link: 0,
            fwd: true,
            lat: 0.0,
            state: State::Idle,
            time: 0.0,
            from: Vec::new(),
            blend: 0.0,
            cur: Vec::new(),
            loiter: true,
            calm: 0.0,
            seed: hash(id, item),
            plan: None,
            cross: None,
            wait: 0.0,
            poly: NO_POLY,
            travel: Vec3::new(-heading.sin(), 0.0, -heading.cos()),
            path: Vec::new(),
            since_plan: 0,
            scale: 1.0,
            speed: 0.0,
            rail: None,
            stop: 0.0,
            stops: 0,
            slide: None,
            turn_round: false,
            turn_rest: 0.0,
            then_angry: false,
            dodged: None,
            hold: 0.0,
            item: Some(item),
            age: 0.0,
            life,
            blend_len: 0.25,
        });
        true
    }

    /// The people as the parked unspawner sees them: (item, place, older than its least age).
    pub fn item_list(&self) -> Vec<(Option<u32>, Vec3, bool)> {
        self.list.iter().map(|p| (p.item, p.pos, p.age >= p.life)).collect()
    }

    /// The items whose person stands.
    pub fn item_peds(&self) -> Vec<u32> {
        self.list.iter().filter_map(|p| p.item).collect()
    }

    /// The parked unspawner's removal: the person of item `h`.
    pub fn remove_item_person(&mut self, h: u32) {
        self.list.retain(|p| p.item != Some(h));
    }
    pub fn remove_items(&mut self, live: &dyn Fn(u32) -> bool) {
        self.list.retain(|p| p.item.map_or(true, live));
    }

    /// The walkers' LOD by the main view, as the original's walker render routine (sub_82794358, main
    /// pass) gets it from its vt+44 sub_82794308 -> sub_827902F0(the set's render type, the sphere's
    /// centre, radius 0.9 (0x82007FA8), flag 0): -1 when the sphere is outside the view or behind an
    /// occluder or its far side is 500 or more in front of the near plane (0x82003250); LOD 0 below
    /// the set's `fHighToMedSwitch` x `fLodSwitchScale`, LOD 1 below `fMedToLowSwitch` x the scale,
    /// else LOD 2 (every set: 40 / 80). There is no other distance limit. OPTIONS > PERFORMANCE
    /// (`k`): DRAW DISTANCE x the 500, DETAIL LEVEL x the steps (3X: LOD 0 throughout).
    /// The centre: the original takes the walker's matrix x its bound's centroid (sub_826E9F88);
    /// the port takes the walker's root (the centroid was not read - approximate).
    pub fn pick_lods(&mut self, eye: Vec3, axis: Vec3, near: f32, k: crate::props::Lod, frustum: &Frustum, occluders: &crate::occluder::Selected) {
        const RADIUS: f32 = 0.9;
        self.lod_pick.clear();
        for p in &self.list {
            let Some(Some(ty)) = self.types.get(&p.ty) else { continue };
            let d = (p.pos - eye).dot(axis) - near + RADIUS;
            let shown = frustum.intersects_sphere(p.pos, RADIUS) && !occluders.hides_sphere(p.pos, RADIUS);
            self.lod_pick.insert(p.key, crate::vehicles::Vehicles::lod_by(ty.lod_switch, d, shown, k));
        }
    }

    /// Skin everything in view into this frame's vertex buffer.
    pub fn prepare(&mut self, gpu: &Gpu, cam: Vec3, frustum: &Frustum) {
        self.buf.clear();
        let tables = self.night_lights.as_ref().and_then(|_| crate::render::LAMP_TABLES.read().ok()?.clone());
        let mut local: Vec<(Quat, Vec3)> = Vec::new();
        // (the LOD of `pick_lods`; a walker it has not seen - none in a normal frame - by the same
        // rule on the plain distance, without the occluders)
        let k = crate::props::Lod::now();
        for p in self.list.iter_mut() {
            let Some(Some(ty)) = self.types.get(&p.ty) else { continue };
            let dist = p.pos.distance(cam);
            let lod = self.lod_pick.get(&p.key).copied().unwrap_or_else(|| crate::vehicles::Vehicles::lod_by(ty.lod_switch, dist, frustum.intersects_sphere(p.pos, 0.9), k));
            let visible = lod >= 0;
            let (clip, looped) = ty.clip(p.state);
            clip.local(&ty.model.bones, clip.frame(p.time, looped), &mut local);
            if p.blend > 0.0 && p.from.len() == local.len() {
                blend_pose(&mut local, &p.from, (p.blend / p.blend_len).clamp(0.0, 1.0));
            }
            p.cur.clone_from(&local);
            if !visible {
                continue;
            }
            let lod = lod as usize;
            // (at night the four lights of its program, picked and eased as the original's
            // sub_822F2C48 does at the walker's place - only while it is drawn, as there)
            if let (Some((dynamic, frames)), Some(t)) = (&self.night_lights, &tables) {
                let fl = self.fig_lights.entry(p.key).or_insert_with(|| crate::figlight::FigureLight::new(t, p.pos));
                fl.step(t, p.pos, *frames);
                self.buf.next_lanes = Some(fl.lanes(p.pos, dynamic));
            }
            self.buf.emit(&ty.model, lod, &local, Mat4::from_rotation_translation(p.rot, p.pos));
        }
        if self.night_lights.is_some() {
            let alive: HashSet<(usize, u32)> = self.list.iter().map(|p| p.key).collect();
            self.fig_lights.retain(|k, _| alive.contains(k));
        }
        self.buf.upload(gpu);
    }

    /// What the shadow passes draw of the walkers (`draw` with `shadow`), apart from them.
    pub fn shadow_part(&self) -> Option<crate::skin::ShadowSkin> {
        self.buf.shadow_part()
    }

    /// Draw with the pipeline already set (opaque pass or shadow pass).
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, gpu: &'a Gpu, shadow: bool, draws: &mut usize) {
        self.buf.draw(pass, gpu, shadow, draws);
    }

    /// The speech banks' scan (`voice_bank`) on a thread of its own, once: read cold, the bank folders' indexes took
    /// one game tick 313 ms at the first pedestrian's shout (docs/PERFORMANCE.md 2026-10-24).
    pub fn scan_voices_ahead(&mut self, audio_root: &Path) {
        if self.voices.is_some() || self.voices_scan.is_some() {
            return;
        }
        let root = audio_root.to_path_buf();
        self.voices_scan = std::thread::Builder::new().name("speech banks".into()).spawn(move || scan_voice_banks(&root)).ok();
    }

    /// Speech bank of a voice key ("fh_006"), from the converted `ped_*` banks.
    pub fn voice_bank(&mut self, audio_root: &Path, voice: &str, female: bool) -> Option<String> {
        if self.voices.is_none() {
            let m = self.voices_scan.take().and_then(|h| h.join().ok()).unwrap_or_else(|| scan_voice_banks(audio_root));
            log::info!("peds: {} speech banks", m.len());
            self.voices = Some(m);
        }
        let m = self.voices.as_ref()?;
        if let Some(b) = m.get(voice).or_else(|| m.get(voice.trim_start_matches("sc_"))) {
            return Some(b.clone());
        }
        // no bank of its own: borrow one of the same sex
        let g = if female { 'f' } else { 'm' };
        let mut same: Vec<&String> = m.iter().filter(|(k, _)| k.trim_start_matches("sc_").starts_with(g)).map(|(_, v)| v).collect();
        same.sort();
        if same.is_empty() {
            return None;
        }
        Some(same[hash(mcla_formats::hash::joaat(voice), 3) as usize % same.len()].clone())
    }
}

/// The speech bank of each voice key: the folder of the first `ped_*_evade_<voice>_NN` wave in a bank's index.
fn scan_voice_banks(audio_root: &Path) -> HashMap<String, String> {
    let mut m = HashMap::new();
    if let Ok(rd) = std::fs::read_dir(audio_root.join("0x051b41b9")) {
        for e in rd.flatten() {
            let Ok(t) = std::fs::read_to_string(e.path().join("_index.tsv")) else { continue };
            // "ped_slow_evade_fh_006_06" -> "fh_006"
            for line in t.lines().take(40) {
                let name = line.split('\t').next().unwrap_or("");
                let Some(i) = name.find("_evade_") else { continue };
                let rest = &name[i + 7..];
                if let Some(j) = rest.rfind('_') {
                    m.insert(rest[..j].to_string(), format!("0x051b41b9/{}", e.file_name().to_string_lossy()));
                    break;
                }
            }
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ambient::Tod;

    fn quad() -> NavPoly {
        NavPoly {
            chunk: 0,
            material: 0,
            flags: AI_SIDEWALK,
            area: 8.0,
            center: Vec3::new(2.0, 0.0, 1.0),
            normal: Vec3::Y,
            nv: 4,
            verts: [Vec3::new(0.0, 0.0, 0.0), Vec3::new(4.0, 0.0, 0.0), Vec3::new(4.0, 0.0, 2.0), Vec3::new(0.0, 0.0, 2.0)],
            first_link: 0,
            n_links: 0,
        }
    }

    #[test]
    fn random_point_is_inside_and_even() {
        let q = quad();
        let mut halves = [0; 2];
        for i in 0..400 {
            let p = random_point(&q, (i % 20) as f32 / 20.0 + 0.025, (i / 20) as f32 / 20.0 + 0.025);
            assert!(q.contains(p.x, p.z), "{p:?}");
            halves[(p.x >= 2.0) as usize] += 1;
        }
        // area-uniform: about as many on either half
        assert!((halves[0] as i32 - halves[1] as i32).abs() < 40, "{halves:?}");
    }

    #[test]
    fn tod_lookup_like_the_original() {
        // Sunset Strip's pedestrian density table
        let t = Tod(vec![(0, 200, 1.0), (200, 600, 0.2), (600, 1200, 0.5), (1200, 1800, 0.8), (1800, 0, 1.2)]);
        assert_eq!(t.at(1.0), 1.0);
        // HHMM inclusive both ends, the first row wins
        assert_eq!(t.at(2.0), 1.0);
        assert_eq!(t.at(2.5), 0.2);
        assert_eq!(t.at(9.0), 0.5);
        assert_eq!(t.at(23.99), 1.2);
        assert_eq!(Tod::default().at(5.0), 1.0);
    }

    /// F-59108 (static init of the on-the-spot turn rate) run on the original's code and image
    /// (work/verify-people/peds/gen_and_run.py): the original's float is 13 ULP above TURN_RATE
    /// (the constant is the rounded decimal 2.94524; the original divides f32 pi/4 by 8 * f32 1/30).
    #[test]
    fn turn_rate_is_the_originals_static_init() {
        let Ok(t) = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../work/verify-people/peds/turn_reference.txt")) else { return };
        let want = u32::from_str_radix(t.trim(), 16).unwrap();
        assert_eq!(want - TURN_RATE.to_bits(), 13, "port {:08x} original {want:08x}", TURN_RATE.to_bits());
    }

    #[test]
    fn view_cone_and_band() {
        let mut p = Peds::new();
        p.set_view(Vec3::X, 0.0);
        let f = Vec3::ZERO;
        // standing: everything within 95 m but straight across is in view, ahead or behind
        assert!(p.in_view(Vec3::new(60.0, 0.0, 10.0)));
        assert!(p.in_view(Vec3::new(-60.0, 0.0, 10.0)));
        assert!(!p.in_view(Vec3::new(96.0, 0.0, 0.0)));
        // at 45 mph and more the cone is 45 degrees
        p.set_view(Vec3::X, 50.0);
        assert!(!p.in_view(Vec3::new(30.0, 0.0, 31.0)));
        assert!(p.in_view(Vec3::new(30.0, 0.0, 29.0)));
        // unspawn: 50 m to the side, 150 m ahead, 75 m behind (the recording of the original)
        assert!(!p.out_of_range(f, Vec3::new(149.0, 0.0, 0.0)));
        assert!(p.out_of_range(f, Vec3::new(151.0, 0.0, 0.0)));
        assert!(p.out_of_range(f, Vec3::new(-76.0, 0.0, 0.0)));
        assert!(!p.out_of_range(f, Vec3::new(-60.0, 0.0, 40.0)));
        assert!(p.out_of_range(f, Vec3::new(10.0, 0.0, 50.5)));
    }

    #[test]
    fn stop_timer_like_the_original() {
        // sub_8279A8C8 with the idle clip lengths seen live; the stop lengths recorded on the
        // original were 2.07 / 2.40 / 4.40 / 4.73 / 6.73 / 25.4 s ...
        let mut r = [0.0f32, 0.5, 0.9].into_iter().cycle();
        let mut rand = || r.next().unwrap();
        let v = 2.333;
        let got: Vec<f32> = (0..3).map(|_| stop_time(v, &mut rand)).collect();
        for (g, n) in got.iter().zip([1.0f32, 2.0, 3.0]) {
            assert!((g - (n * v - 8.0 / 30.0)).abs() < 1e-4, "{got:?}");
        }
        assert!((stop_time(25.667, &mut rand) - 25.4).abs() < 1e-3);
        let t = stop_time(0.0, &mut rand);
        assert!((2.0..4.0).contains(&t));
    }

    #[test]
    fn rails_go_midpoint_to_midpoint() {
        let path: Vec<(Vec3, u32)> = (0..5).map(|k| (Vec3::new(k as f32 * 3.0, 0.0, 0.0), 0)).collect();
        // on segment 0 facing along it: its middle is 1.5 m ahead
        let m = next_rail(&path, Vec3::new(0.2, 0.0, 0.0), Vec3::X).unwrap();
        assert!((m.x - 1.5).abs() < 1e-4);
        // at that middle: the next one (the nearest is closer than 0.5 m)
        let m = next_rail(&path, Vec3::new(1.5, 0.0, 0.0), Vec3::X).unwrap();
        assert!((m.x - 4.5).abs() < 1e-4);
        // facing back: only the fallback of 216 degrees for the nearest segment remains
        let m = next_rail(&path, Vec3::new(0.2, 0.0, 0.0), -Vec3::X).unwrap();
        assert!((m.x - 1.5).abs() < 1e-4);
        // at the end of the path: nothing
        assert!(next_rail(&path, Vec3::new(10.5, 0.0, 0.0), Vec3::X).is_none());
    }

    #[test]
    fn car_prediction_over_a_second() {
        let car = Threat { pos: Vec3::new(-10.0, 0.0, 0.0), vel: Vec3::new(20.0, 0.0, 0.0) };
        // in its path within the next second (four steps of 0.25 s)
        assert!(car_threat(Vec3::new(1.0, 0.0, 0.8), &[car]).is_some());
        // beside its path, behind it, or further than a second away
        assert!(car_threat(Vec3::new(1.0, 0.0, 3.0), &[car]).is_none());
        assert!(car_threat(Vec3::new(-15.0, 0.0, 0.0), &[car]).is_none());
        assert!(car_threat(Vec3::new(15.0, 0.0, 0.0), &[car]).is_none());
        // the way out is away from its line
        let (_, away, _) = car_threat(Vec3::new(1.0, 0.0, -0.8), &[car]).unwrap();
        assert!(away.z < -0.9);
    }

    #[test]
    fn furniture_boxes() {
        let o: Obst = (Vec3::new(10.0, 0.0, 0.0), Vec3::X, glam::Vec2::new(1.0, 0.25));
        assert_eq!(box_dist(&o, Vec3::new(10.5, 0.0, 0.1)), 0.0);
        assert!((box_dist(&o, Vec3::new(10.0, 0.0, 1.25)) - 1.0).abs() < 1e-5);
        assert!((box_dist(&o, Vec3::new(12.0, 0.0, 0.0)) - 1.0).abs() < 1e-5);
        // a walker 0.9 m off a post is seen by the 0.667 m sphere only within reach
        assert!(sphere_hit(Vec3::new(10.0, 0.0, 0.8), AWARE_RADIUS, 0, &[], &[o], false).is_some());
        assert!(sphere_hit(Vec3::new(10.0, 0.0, 1.0), AWARE_RADIUS, 0, &[], &[o], false).is_none());
    }

    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc"]
    fn spawn_target_at_sunset_strip() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let root = Path::new(&dir);
        let mesh = NavMesh::parse(&load_resource_file(&root.join("resources/city/sc/0x69d17d57.r26")).unwrap()).unwrap();
        // the recorded original: player car standing at Sunset Strip, 116 candidate
        // polygons of 5351.95 m^2, density 0.5 (06:00-12:00), 30 pedestrians
        let focus = Vec3::new(-638.9, 18.2, -636.8);
        // the chunks' quadtrees (27 of 35 chunks have one), and the query through them finds
        // every polygon a plain search finds
        assert_eq!(mesh.quads.iter().filter(|q| q.is_some()).count(), 27);
        let all = mesh.sphere_query(focus, 140.0, usize::MAX, |_, q| q.sidewalk() && q.nv >= 3 && q.closest(focus.x, focus.z).distance(focus) <= 140.0);
        let mut plain = Vec::new();
        mesh.near(focus, 140.0, |i, q| {
            if q.sidewalk() && q.nv >= 3 && q.closest(focus.x, focus.z).distance(focus) <= 140.0 {
                plain.push(i);
            }
        });
        plain.sort_unstable();
        assert_eq!(all, plain);
        let c = spawn_candidates(&mesh, focus, 140.0);
        let area: f32 = c.iter().map(|x| x.1).sum();
        assert_eq!(c.len(), 116);
        assert!((area - 5351.95).abs() < 1.0, "{area}");
        let d = Distribution::load(root, "ped_");
        let target = area * 0.011 * d.density_at(focus, 9.0);
        assert!((target - 29.44).abs() < 0.05, "{target}");
        // a random walk from there stays on the pavement
        let start = mesh.find(Vec3::new(-640.0, 18.0, -640.0), AI_SIDEWALK, 3.0).or(c.first().map(|x| x.0)).unwrap();
        let mut r = 7u32;
        let mut rand = || {
            r = r.wrapping_mul(1664525).wrapping_add(1013904223);
            (r >> 8) as f32 / 16777216.0
        };
        let path = random_path(&mesh, start, mesh.polys[start as usize].center, Vec3::X, &mut rand);
        assert!(!path.is_empty());
        assert!(path.iter().all(|h| mesh.polys[h.2 as usize].sidewalk()));
    }
}
