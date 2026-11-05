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

use crate::ambient::{xml_bool, xml_value, Distribution};
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
    /// the set's run clip (its clip 2), else the walk clip
    run: Clip,
    /// the clips' lengths and distances, idle / walk / run (the anim set's A28 / A36 as F-33058 reads them; here the
    /// port's clips' duration and root travel)
    a28: [f32; 3],
    a36: [f32; 3],
    evade: Vec<Clip>,
    /// the avoid list (file order) with each entry's clip in `evade`: what `evade_pick` reads once the car dodge
    /// (F-15352, docs/reference/people/car_dodge_rule.md) is ported
    #[allow(dead_code)]
    avoid: Vec<AvoidItem>,
    angry: Vec<Clip>,
    /// the shared Pivot clips (`Mped_turnL/R_90/180`) with the yaw their root turns
    pivot: Vec<(Clip, f32)>,
    lod_switch: [f32; 2],
    /// `fMinTurningRadius` of the set's `mcAmbientType.xml` (type +160)
    turn_radius: f32,
}

impl PedType {
    fn clip(&self, s: State) -> (&Clip, bool) {
        match s {
            State::Walk => (&self.walk, true),
            State::Idle => (&self.idle, true),
            State::Run => (&self.run, true),
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
    /// locomotion state 2 (F-33058): a slide faster than the middle of the walk and run clips' speeds
    Run,
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
    /// the avoidance part's dodge words (M, F-15352 / F-83234 / F-56160)
    dodge: Avoid,
    /// a layout item's person (F-39915): its item, and its age and least age (8 + 4U s)
    item: Option<u32>,
    age: f32,
    life: f32,
    /// the length of the blend running (`blend` counts it down)
    blend_len: f32,
    /// U, the forward ring, sphere 0 and the target facing (F-49074)
    ground: Ground,
    /// the path rail ridden (F-61656) and its phase
    ride: Option<(crate::tcp_rail::TcpRail, f32)>,
    /// the clip slots (+4784 .. +4884): the locomotion clip playing, its rate, the one-off clip (F-29547 / F-52026)
    clips: ClipSlots,
}

const NO_POLY: u32 = u32::MAX;
/// The original's frame: path requests are let through once per 30 Hz frame; spawning and
/// unspawning run on every other one (F-47889 alternates by frame parity).
const FRAME: f32 = 1.0 / 30.0;
/// The spawner's occlusion test (F-95989): a sphere of 2 m, 1.5 m above the spawn
/// point, against the volumes seen from 1.75 m above the player (view 2, F-75502).
const OCC_LIFT: f32 = 1.5;
const OCC_RADIUS: f32 = 2.0;
const OCC_EYE: f32 = 1.75;
/// The occluder range scale the city sets up (F-20115 writes 10 to 0x827D2958).
const OCC_RANGE_SCALE: f32 = 10.0;

fn flat(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
}

/// The original's random walk over the sidewalk polygons (`navQueryPathRandom`, search
/// F-28561, set up by F-39482). Up to 32 hops; at each hop every sidewalk
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
    // finds its way back (F-50927)
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
/// Sphere refresh (A-72290 via the manager): every 30 Hz frame each walker re-queries,
/// with this chance by its distance to the focus (within 25 m, 85 m, beyond), the one of its
/// three spheres the global frame counter points at (counter % 3): 0 the ground probe (r 2,
/// mask 0x800), 1 around the walker, 2 ahead of it. The hits only last that frame.
const AWARE_CHANCE: [(f32, f32); 2] = [(25.0, 0.4), (85.0, 0.15)];
const AWARE_CHANCE_FAR: f32 = 0.1;
/// Sidestep: 0.25 s, the push tried straight and 45 degrees either way (table 0x820da334).
const SLIDE_TIME: f32 = 0.25;
/// Clip blends are 8 frames (0x82830878).
const BLEND_S: f32 = 8.0 / 30.0;
/// Turning on the spot: at most pi/4 per 8 frames, the bits the original's static initialiser F-59108 stores
/// (2.9452431; tools/ped_walk_reference.py runs it).
const TURN_RATE: f32 = f32::from_bits(0x403C_7EDD);
/// A rail around something in the way: at most 130 degrees round (F-92334); the way
/// to it swept by a capsule of the ped's radius 0.6 - 1.2 m up (F-23371, mask 0x383e:
/// furniture, cars, peds and buildings, the walker being passed included).
const AVOID_TURN: f32 = 2.26893;
const AVOID_CLEAR: f32 = PED_RADIUS;

// ---- The walker's ground, up vector and drawn matrix: its slot 80 F-49074 (sphere refresh, ground, matrix tail), the
// type's slot 64 F-24029 (the ground hit, U, the place) and slot 60 F-59837 (set sphere k), the forward ring F-89721 /
// F-22858. Vector-unit steps flush denormals in and out, their multiply-add is two roundings, the three-lane dot is
// (0 + z z') + (y y' + x x'); scalar `fmadds` is fused. Decode and evidence: docs/reference/people/ped_ground_rule.md
// (tools/ped_ground_reference.py).

/// The ground sphere (sphere 0): radius 2.0, drawn on the frames whose counter points at it (F-49074).
const GROUND_RADIUS: f32 = 2.0;
/// A ground contact counts with normal.y above this (0x3F3504F3).
const GROUND_MIN_NY: f32 = 0.70710677;
/// Contacts this close in plan (squared, m^2) to the best so far are a tie: the higher wins.
const GROUND_TIE: f32 = 0.25;
/// The city bound's type bit (the other instance's archetype flags).
const GROUND_TYPE: u32 = 0x800;

#[inline]
fn vftz(x: f32) -> f32 {
    if x != 0.0 && x.is_subnormal() { 0.0f32.copysign(x) } else { x }
}

fn vlanes(a: Vec3, b: Vec3, f: impl Fn(f32, f32) -> f32) -> Vec3 {
    Vec3::new(vftz(f(vftz(a.x), vftz(b.x))), vftz(f(vftz(a.y), vftz(b.y))), vftz(f(vftz(a.z), vftz(b.z))))
}

/// vmsum3fp a . b: (0 + z z') + (y y' + x x'), each step flushed.
fn vdot3(a: Vec3, b: Vec3) -> f32 {
    let p = vlanes(a, b, |x, y| x * y);
    vftz(vftz(0.0 + p.z) + vftz(p.y + p.x))
}

/// vsubfp 0 - v.
fn vneg(v: Vec3) -> Vec3 {
    vlanes(Vec3::ZERO, v, |x, y| x - y)
}

/// vnmsubfp -(a x splat(k) - c).
fn vnmsub(a: Vec3, k: f32, c: Vec3) -> Vec3 {
    -vlanes(vlanes(a, Vec3::splat(k), |x, y| x * y), c, |x, y| x - y)
}

/// One contact of sphere 0 as a hit slot holds it: the other instance's archetype type flags, the contact point
/// (slot +16) and the normal (slot +48).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GroundContact {
    pub kind: u32,
    pub point: Vec3,
    pub normal: Vec3,
}

/// F-24029: of sphere 0's hit slots (None: empty) the one of type 0x800 with normal.y > 0.70710677 nearest in plan to
/// the sphere's centre, where one within 0.25 m^2 of the best so far wins only by being higher (so the order counts).
/// Nothing when slot 0 is empty. Gives (slot, N, D) with D = N (N . (contact - centre)): the place is T + D and U = N.
pub(crate) fn ground_pick(slots: &[Option<GroundContact>], centre: Vec3) -> Option<(usize, Vec3, Vec3)> {
    slots.first()?.as_ref()?;
    let (mut best, mut besty, mut n, mut k) = (f32::MAX, -f32::MAX, Vec3::ZERO, None);
    for (j, s) in slots.iter().enumerate() {
        let Some(s) = s else { continue };
        if s.kind & GROUND_TYPE == 0 || !(s.normal.y > GROUND_MIN_NY) {
            continue;
        }
        let dx = s.point.x - centre.x;
        let dz = s.point.z - centre.z;
        let d = crate::physics::exact::fma(dz, dz, dx * dx);
        let take = if best == f32::MAX {
            d < best
        } else if (d - best).abs() < GROUND_TIE {
            s.point.y > besty
        } else {
            d < best
        };
        if take {
            besty = s.point.y;
            best = d;
            n = s.normal;
            k = Some(j);
        }
    }
    let k = k?;
    let v = crate::tcp_rail::vdiff(slots[k].unwrap().point, centre);
    let q = crate::physics::exact::fma(v.y, n.y, crate::physics::exact::fma(v.z, n.z, v.x * n.x));
    Some((k, n, crate::tcp_rail::vmul3(n, q)))
}

/// F-49074 after the pick: the out place and facing row r2 from the target T (its r2 and place), the drawn place `cur`,
/// the walker's speed (ped +412) and the frame's dt. `up` is U (ped +4768): a pick sets it to the ground normal.
/// In a one-off clip (behaviour state 3) only y comes from the ground (T + D with a hit, else `cur`), x and z from T;
/// otherwise a hit places the walker at T + D; without one it goes on from `cur` along T's facing with its part along U
/// taken out, made unit, at speed x dt, and faces that way.
pub(crate) fn ground_place(t_r2: Vec3, t_place: Vec3, cur: Vec3, up: &mut Vec3, pick: Option<(usize, Vec3, Vec3)>, speed: f32, dt: f32, clip: bool) -> (Vec3, Vec3) {
    if let Some((_, n, _)) = pick {
        *up = n;
    }
    match (clip, pick) {
        (true, p) => {
            let y = p.map_or(cur.y, |(_, _, d)| crate::tcp_rail::vadd3(t_place, d).y);
            (Vec3::new(t_place.x, y, t_place.z), t_r2)
        }
        (false, Some((_, _, d))) => (crate::tcp_rail::vadd3(t_place, d), t_r2),
        (false, None) => {
            let a = vneg(t_r2);
            let w = vnmsub(*up, vdot3(*up, a), a);
            let f = crate::tcp_rail::vnorm(w);
            (crate::tcp_rail::vmadd3(f, speed * dt, cur), vneg(f))
        }
    }
}

/// The walker's forward ring (ped +4608): the last 8 frames' facing rows r2, their count, the next slot and the
/// running sum. Pushed every frame (F-22858); reset to the current row while a one-off clip runs (ped +4804, F-89721).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct FwdRing {
    pub count: i32,
    pub index: i32,
    pub sum: Vec3,
    pub e: [Vec3; 8],
}

impl FwdRing {
    /// F-89721: count 8, next 0, sum v x 8, every entry v.
    pub(crate) fn reset(&mut self, v: Vec3) {
        self.count = 8;
        self.index = 0;
        self.sum = crate::tcp_rail::vmul3(v, 8.0);
        self.e = [v; 8];
    }

    /// F-22858: the first entry starts the sum; up to 8 are added; then the oldest is taken out of the sum and replaced.
    pub(crate) fn push(&mut self, v: Vec3) {
        if self.count == 0 {
            self.sum = v;
            self.e[0] = v;
            self.index = 1;
            self.count = 1;
        } else if self.count < 8 {
            self.e[(self.index & 7) as usize] = v;
            self.sum = crate::tcp_rail::vadd3(self.sum, v);
            self.count += 1;
            self.index += 1;
        } else {
            if self.index == 8 {
                self.index = 0;
            }
            let i = (self.index & 7) as usize;
            self.sum = crate::tcp_rail::vdiff(self.sum, self.e[i]);
            self.e[i] = v;
            self.sum = crate::tcp_rail::vadd3(self.sum, v);
            self.index += 1;
        }
    }

    /// F-49074's tail: the drawn rows r0, r2 (r1 is world up): the mean (sum x the refined reciprocal of the count)
    /// with y set to 0, made unit; r0 = r1 x r2 as the vector unit has it.
    pub(crate) fn rows(&self) -> (Vec3, Vec3) {
        let m = if self.count > 0 {
            let c = self.count as f32;
            let r = vftz(1.0 / c);
            let e = -vftz(vftz(r * c) - 1.0);
            crate::tcp_rail::vmul3(self.sum, vftz(vftz(r * e) + r))
        } else {
            Vec3::ZERO
        };
        let r2 = crate::tcp_rail::vnorm(Vec3::new(m.x, 0.0, m.z));
        let r1 = Vec3::Y;
        // -(r2.yzx r1.zxy - r2.zxy r1.yzx)
        let p = vlanes(Vec3::new(r2.z, r2.x, r2.y), Vec3::new(r1.y, r1.z, r1.x), |x, y| x * y);
        let q = vlanes(Vec3::new(r2.y, r2.z, r2.x), Vec3::new(r1.z, r1.x, r1.y), |x, y| x * y);
        (-vlanes(q, p, |x, y| x - y), r2)
    }
}

/// Sphere 0's contacts as the port finds them: every city-bound triangle (the collider's sectors: the city bound and
/// roads, type 0x800) whose closest point to the centre lies within the radius, in the collider's order; the contact is
/// that closest point, the normal the face's own (front side). (The original's sphere test keeps 12 slots, filled in an
/// order not decoded; see the doc.)
fn ground_contacts(col: &WorldCollider, c: Vec3, r: f32) -> Vec<Option<GroundContact>> {
    let mut out = Vec::new();
    col.triangles_of(c - Vec3::splat(r), c + Vec3::splat(r), |s, i| {
        let t = &s.tris[i];
        let q = crate::physics::collision::closest_point_tri(c, t);
        if (q - c).length_squared() <= r * r {
            let n = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
            out.push(Some(GroundContact { kind: GROUND_TYPE, point: q, normal: n }));
        }
    });
    out
}

/// The walker's ground state (F-49074): U, the forward ring, sphere 0 (its centre +48, pending +20, the hits of its last
/// test), the one-off clip flag (ped +4804) as it was at the frame's start, and the facing of the rail (the port's: the
/// behaviour's target facing, kept apart from the drawn one).
#[derive(Clone, Debug, Default)]
struct Ground {
    up: Vec3,
    ring: FwdRing,
    centre: Vec3,
    pending: bool,
    hits: Vec<Option<GroundContact>>,
    one_off: bool,
    face: Vec3,
    /// this frame's target facing (the rail's, or the push's while sliding aside)
    t_face: Vec3,
}

/// F-49074 for a walker on the mesh, once per update of the port, after its behaviour step (`p.pos`, `p.ground.t_face`:
/// the target T) and a one-off clip's root motion (T is then where the clip took it). `cur`: where it was drawn before;
/// `frames`: the 30 Hz manager frames this update covers; `draw0`: sphere 0 drawn in them. The order is the original's
/// manager frame (F-96156): at the frame's start every sphere's slots are cleared and a pending sphere is tested
/// (F-12278 -> F-78763: F-96720 clears, F-66549 queues, F-34785 clears the pending flag); the queued tests run on
/// the 8 worker jobs (F-77931) and are waited for (F-29302) before the walkers' updates, where slot 80 draws (F-59837:
/// the centre is T's place) and reads sphere 0. So a draw is read once, in the next frame, against the centre of the
/// draw. The port's update may cover no manager frame (no test, no ring step) or several (one test; the ring stepped
/// once a frame).
fn walker_ground(p: &mut Ped, col: &WorldCollider, cur: Vec3, frames: u32, draw0: bool, dt: f32) {
    let clip = matches!(p.state, State::Evade(_) | State::Angry(_) | State::Turn(_));
    let g = &mut p.ground;
    if clip {
        // (a one-off clip's T is the drawn matrix moved by the clip: F-94626 for state 3)
        g.face = flat(p.rot * -Vec3::Z).normalize_or(Vec3::Z);
        g.t_face = g.face;
    }
    let face = if g.t_face != Vec3::ZERO { g.t_face } else { flat(p.rot * -Vec3::Z).normalize_or(Vec3::Z) };
    // (the port's rails are flat: T's height is where the walker was drawn; the original's is the rail's, F-74567)
    let t_place = if clip { p.pos } else { Vec3::new(p.pos.x, cur.y, p.pos.z) };
    if frames > 0 {
        g.hits.clear();
        if g.pending {
            g.hits = ground_contacts(col, g.centre, GROUND_RADIUS);
            g.pending = false;
        }
    }
    if draw0 {
        g.centre = t_place;
        g.pending = true;
    }
    let pick = ground_pick(&g.hits, g.centre);
    g.hits.clear();
    let (place, r2) = ground_place(-face, t_place, cur, &mut g.up, pick, p.speed, dt, clip);
    let steps = if g.ring.count == 0 { frames.max(1) } else { frames }.min(8);
    for _ in 0..steps {
        if g.one_off {
            g.ring.reset(r2);
        } else {
            g.ring.push(r2);
        }
    }
    let (r0, r2) = g.ring.rows();
    if r2 != Vec3::ZERO {
        p.rot = Quat::from_mat3(&glam::Mat3::from_cols(r0, Vec3::Y, r2));
    }
    // (ped +4804 for the next frame: set by F-20899 after slot 80 when a clip starts, cleared by F-52026)
    g.one_off = clip;
    p.pos = place;
}

// ---- The walker's path rail (F-61656 with the nearest segment F-93741, the closest point F-17399 and the rail to a
// segment's middle F-16360), its locomotion state (F-33058 with the slide speed F-92375), the anim update's clip choice
// (F-29547) and the clip start (F-52026). Decode and evidence: docs/reference/people/ped_walk_rule.md
// (tools/ped_walk_reference.py).

/// F-61656's turn limits and least length: a rail to the middle of segment
/// i, i+1, i+2, i-1, i-2 turning at most 1.2566371 in all, longer than 0.5 m; else segment i turning at most 3.7699115.
const RAIL_TURN: f32 = f32::from_bits(0x3FA0_D97C);
const RAIL_TURN_LAST: f32 = f32::from_bits(0x4071_463B);
const RAIL_MIN: f32 = 0.5;

/// F-17399: the point of segment a -> b nearest to q (3-D; t clamped to 0 .. 1, 0 for a zero segment).
fn closest_on_segment(a: Vec3, b: Vec3, q: Vec3) -> Vec3 {
    seg_nearest(a, b, q).1
}

/// F-93741: the path segment nearest to q (3-D squared distance, the first of equals), None for fewer than 2 points.
pub(crate) fn nearest_segment(pts: &[Vec3], q: Vec3) -> Option<usize> {
    use crate::physics::exact::fma;
    let mut best = (f32::MAX, None);
    for i in 0..pts.len().saturating_sub(1) {
        let c = closest_on_segment(pts[i], pts[i + 1], q);
        let (dy, dz, dx) = (c.y - q.y, c.z - q.z, c.x - q.x);
        let d = fma(dx, dx, fma(dz, dz, dy * dy));
        if d < best.0 {
            best = (d, Some(i));
        }
    }
    best.1
}

/// F-16360: the rail from `p` heading `h0` to the middle T of segment i, arriving with the flat heading P -> T: of the
/// four arc-line-arc rails at radius `r` (senses (0, 0), (0, 1), (1, 0), (1, 1), F-85722) the shortest valid one whose
/// total turn is at most `max_turn`; None when there is none.
pub(crate) fn rail_to_mid(pts: &[Vec3], i: usize, p: Vec3, h0: Vec3, r: f32, max_turn: f32) -> Option<crate::tcp_rail::TcpRail> {
    use crate::tcp_rail::{build, vadd3, vdiff, vmul3, vnorm};
    let t = vmul3(vadd3(pts[i], pts[i + 1]), 0.5);
    let d = vdiff(t, p);
    let h1 = vnorm(Vec3::new(d.x, 0.0, d.z));
    let mut best = (f32::MAX, None);
    for (s0, s1) in [(false, false), (false, true), (true, false), (true, true)] {
        let q = build(p, h0, t, h1, r, s0, s1);
        if q.valid() && !(q.total_turn() > max_turn) {
            let l = q.len();
            if !(l >= best.0) {
                best = (l, Some(q));
            }
        }
    }
    best.1
}

/// F-61656: the next rail along the path from `p` heading `h0`, turning radius `r` (the type's `fMinTurningRadius`):
/// from the nearest segment i, the first of i, i+1, i+2, i-1, i-2 (those that exist) with a rail longer than 0.5 m
/// within 1.2566371 of turn; else segment i within 3.7699115, any length. None: the caller gives the path up
/// (F-95960: an invalid rail drops the path).
pub(crate) fn next_rail(pts: &[Vec3], p: Vec3, h0: Vec3, r: f32) -> Option<crate::tcp_rail::TcpRail> {
    let i = nearest_segment(pts, p)?;
    let n = pts.len() - 1;
    let tries = [Some(i), (i + 1 < n).then_some(i + 1), (i + 2 < n).then_some(i + 2), i.checked_sub(1), i.checked_sub(2)];
    for k in tries.into_iter().flatten() {
        if let Some(q) = rail_to_mid(pts, k, p, h0, r, RAIL_TURN) {
            if q.len() > RAIL_MIN {
                return Some(q);
            }
        }
    }
    rail_to_mid(pts, i, p, h0, r, RAIL_TURN_LAST)
}

/// F-92375: the slide's speed, its plan distance over its time (at least 1e-6 s).
fn slide_speed(from: Vec3, to: Vec3, time: f32) -> f32 {
    let (dx, dz) = (from.x - to.x, from.z - to.z);
    let den = if f32::from_bits(0x3586_37BD) - time >= 0.0 { f32::from_bits(0x3586_37BD) } else { time };
    crate::physics::exact::fma(dz, dz, dx * dx).sqrt() / den
}

/// The anim set's clip speeds: distance A36[k] over length A28[k] (0 for a zero length); clip 0 idle, 1 walk, 2 run.
fn clip_speed(a28: &[f32; 3], a36: &[f32; 3], k: usize) -> f32 {
    if a28[k] != 0.0 { a36[k] / a28[k] } else { 0.0 }
}

/// F-33058: the locomotion state (behaviour +1360) and the wanted speed with its lag (the request's +152 / +156, 0.1 s).
/// `flags` = behaviour +532 (0x2 a clip playing, 0x4 sliding), `stop` its stop timer, `rail` a current rail,
/// `scale` the walker's speed scale, `clip` the avoidance clip (F-71036) present. Not sliding or in a clip: 0 idle
/// (stop timer above 0 or no rail) or 1 walk at scale x the walk clip's speed. Else 3 with a clip; sliding: 1 walk or 2
/// run by the slide's speed against the middle of the walk and run clips' speeds, wanting the slide's speed.
pub(crate) fn locomotion(flags: u32, stop: f32, rail: bool, scale: f32, clip: bool, slide: impl FnOnce() -> f32, a28: &[f32; 3], a36: &[f32; 3]) -> (u32, f32, f32) {
    let lag = f32::from_bits(0x3DCC_CCCD);
    if flags & 4 == 0 && flags & 2 == 0 {
        if stop > 0.0 || !rail {
            return (0, 0.0, lag);
        }
        return (1, scale * clip_speed(a28, a36, 1), lag);
    }
    if clip {
        return (3, 0.0, lag);
    }
    let s = slide();
    let mid = (clip_speed(a28, a36, 2) + clip_speed(a28, a36, 1)) * 0.5;
    (if s > mid { 2 } else { 1 }, s, lag)
}

/// What the walker's anim update F-29547 asks of the clip player for the locomotion state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum AnimCmd {
    Keep,
    /// the one-off clip (F-20899) this clip
    OneOff(u32),
    /// start clip k at this rate (F-52026)
    Start(i32, f32),
    /// the current clip's rate (F-66231)
    Rate(f32),
}

/// F-29547's switch on the locomotion state `st`: 0 -> clip 0 at rate 1 unless it plays; 3 -> the avoidance clip as a
/// one-off unless it is the one playing; 1 / 2 -> clip 1 / 2 at the speed scale, sliding at the slide's speed over the
/// clip's, clamped to 0.75 .. 1.25; the clip playing gets the rate when it differs (walk) or is
/// higher (run: a lower one is kept). `cur` = +4788, `rate` = +4880, `oneoff` = +4804, `clip` = F-71036's.
pub(crate) fn anim_choice(st: u32, cur: i32, rate: f32, oneoff: u32, clip: u32, flags: u32, scale: f32, slide: impl FnOnce() -> f32, a28: &[f32; 3], a36: &[f32; 3]) -> AnimCmd {
    match st {
        0 if cur != 0 => AnimCmd::Start(0, 1.0),
        3 if clip != oneoff => AnimCmd::OneOff(clip),
        1 | 2 => {
            let k = st as usize;
            let mut f = scale;
            if flags & 4 != 0 {
                let r = slide() / clip_speed(a28, a36, k);
                let r = if r - 0.75 >= 0.0 { r } else { 0.75 };
                f = if r - 1.25 >= 0.0 { 1.25 } else { r };
            }
            if cur != k as i32 {
                AnimCmd::Start(k as i32, f)
            } else if (k == 1 && f != rate) || (k == 2 && !(f <= rate)) {
                AnimCmd::Rate(f)
            } else {
                AnimCmd::Keep
            }
        }
        _ => AnimCmd::Keep,
    }
}

/// The walker's clip slots (+4784 previous, +4788 current, +4792 parked, +4804 the one-off clip, +4880 / +4884 the
/// rates), as the walker's constructor F-12432 sets them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ClipSlots {
    pub prev: i32,
    pub cur: i32,
    pub parked: i32,
    pub oneoff: u32,
    pub rate: f32,
    pub prev_rate: f32,
}

impl Default for ClipSlots {
    fn default() -> Self {
        ClipSlots { prev: -1, cur: -1, parked: -1, oneoff: 0, rate: 1.0, prev_rate: 1.0 }
    }
}

/// How F-52026 puts the new clip into the motion tree.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ClipStart {
    /// the clip is not resident: its index parked, nothing played
    Parked,
    /// straight into the tree: the new clip replaces whatever played, no fade
    Direct,
    /// a crmtRequestBlend of weight 0 rising linearly at 30 / 8 per second (8 frames) from what played
    Blend,
}

/// The crossfade's rate (F-52026: 30 / 8, two constants of the image): the new clip's weight w += 3.75 dt, linear.
pub(crate) const CLIP_BLEND_RATE: f32 = 30.0 / 8.0;

/// F-52026: start clip `idx` from t = 0 at `rate`, looped, through the walker's two frame filters (+4888, +4892).
/// Blend when a one-off clip plays, or a clip plays and none is parked; straight in when nothing plays (the first clip)
/// or one is parked. Then: previous = current, current = idx, the rates shift, the one-off is cleared.
pub(crate) fn clip_start(s: &mut ClipSlots, idx: i32, rate: f32, resident: bool) -> ClipStart {
    let k = if !resident {
        s.parked = idx;
        ClipStart::Parked
    } else {
        let k = if s.oneoff != 0 || (s.cur != -1 && s.parked == -1) { ClipStart::Blend } else { ClipStart::Direct };
        s.parked = -1;
        k
    };
    s.prev = s.cur;
    s.cur = idx;
    s.prev_rate = s.rate;
    s.oneoff = 0;
    s.rate = rate;
    k
}

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
/// test (F-23371).
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

/// The wait timer (F-84843) from the idle clip's length `v`.
fn stop_time(v: f32, rand: &mut dyn FnMut() -> f32) -> f32 {
    if v <= 0.0 {
        2.0 + 2.0 * rand()
    } else if v < 5.0 {
        ((rand() * 3.0) as u32 % 3 + 1) as f32 * v - BLEND_S
    } else {
        v - BLEND_S
    }
}

/// A car about to run the walker over (F-15352): its path over the next second in
/// four steps of 0.25 s passing within its half width and the walker's radius. Returns the
/// car's position and the way out (away from the nearest point of its path).
/// One entry of the avoid list (`mcAvoidAnimMgrData.xml`, in file order): `AvoidType` (0 AvoidAny, 1 AvoidMale,
/// 2 AvoidFemale, 3 Pivot, 4 AngryMale, 5 AngryFemale), `ApproachDir` (0 Any, 1 Front, 2 Rear, 3 Left, 4 Right) and
/// the clip, if it was found; `AudioType` (0 None, 1 Fast, 2 Slow: the slide start's speech, F-83234).
#[derive(Clone, Copy, Debug)]
pub struct AvoidItem {
    pub ty: u32,
    pub dir: u32,
    pub clip: Option<usize>,
    pub audio: u32,
}

/// The evade clip pick (F-37443): over the list in order, an entry of the walker's sex (or AvoidAny) whose approach
/// sector holds `approach` (Front |a| <= pi/4, Rear |a| >= 3pi/4, Right |a - pi/2| <= pi/4, Left |a + pi/2| <= pi/4),
/// whose clip has moved at least `min` by `time` (floored at 0.25 s), whose move then points within 50 degrees of
/// `dir` (its angle from -z, F-32060 `aiv::ang_xz`, the difference wrapped into -pi..pi), and that has moved at most `max` by
/// its end; one of those, uniformly by the shared MWC (`index`). `disp(clip, t)` is the clip's mover at `t`
/// (`None`: its end). Returns the entry and its wrapped angle.
#[allow(clippy::too_many_arguments)]
pub fn evade_pick(items: &[AvoidItem], female: bool, dir: f32, approach: f32, time: f32, min: f32, max: f32, mut disp: impl FnMut(usize, Option<f32>) -> Vec3, rng: &mut crate::vehicles::parked::Mwc) -> Option<(usize, f32)> {
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};
    let min2 = min * min;
    let max2 = max * max;
    let t = if time - 0.25 >= 0.0 { time } else { 0.25 };
    let len2 = |d: Vec3| d.z.mul_add(d.z, d.x.mul_add(d.x, d.y * d.y));
    let mut ok = Vec::new();
    for (i, it) in items.iter().enumerate() {
        let Some(c) = it.clip else { continue };
        if it.ty != 0 && it.ty != 1 + female as u32 {
            continue;
        }
        let out = match it.dir {
            1 => approach.abs() > FRAC_PI_4,
            2 => approach.abs() < f32::from_bits(0x4016_CBE4),
            4 => (approach - FRAC_PI_2).abs() > FRAC_PI_4,
            3 => (approach + FRAC_PI_2).abs() > FRAC_PI_4,
            _ => false,
        };
        if out {
            continue;
        }
        let d = disp(c, Some(t));
        if len2(d) < min2 {
            continue;
        }
        let a = crate::aiv::ang_xz(d, Vec3::new(-0.0, -0.0, -1.0)) - dir;
        let a = if -(a - PI) >= 0.0 { a } else { a - TAU };
        let a = if a + PI >= 0.0 { a } else { a + TAU };
        if a.abs() > f32::from_bits(0x3F5F_66F3) {
            continue;
        }
        if len2(disp(c, None)) > max2 {
            continue;
        }
        ok.push((i, a));
    }
    if ok.is_empty() {
        return None;
    }
    Some(ok[rng.index(ok.len() as u32) as usize])
}

// ---------------------------------------------------------------------------------------------
// The walkers' car dodge (docs/reference/people/car_dodge_rule.md): the threat F-15352, the direction search F-94813
// with its probe F-78951 and lane limit F-50861, and the slide start F-83234. Vector operations as the recompiled
// VMX code does them (each lane rounded and flushed, `vlanes`), scalar ones as its FPU code (fused where fused).

fn vadd(a: Vec3, b: Vec3) -> Vec3 {
    vlanes(a, b, |x, y| x + y)
}

fn vsub(a: Vec3, b: Vec3) -> Vec3 {
    vlanes(a, b, |x, y| x - y)
}

fn vmul(a: Vec3, b: Vec3) -> Vec3 {
    vlanes(a, b, |x, y| x * y)
}

/// vmaddfp a x splat(k) + c (the product rounded, then the sum).
fn vmadd(a: Vec3, k: f32, c: Vec3) -> Vec3 {
    vadd(vmul(a, Vec3::splat(k)), c)
}

/// The y lane cleared by `vsel` (the other lanes as they are).
fn vflat(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
}

/// vrsqrtefp as the recompiled code does it: 1 / sqrt(x), each step rounded and flushed.
fn vrsqrt(x: f32) -> f32 {
    vftz(1.0 / vftz(vftz(x).sqrt()))
}

/// The dodge's Newton step on y0 = vrsqrt(x), in vector operations: y0 + (-(x y0^2 - 1)) (0.5 y0).
fn vnr(x: f32, y0: f32) -> f32 {
    let k = -vftz(vftz(x * vftz(y0 * y0)) - 1.0);
    vftz(vftz(k * vftz(0.5 * y0)) + y0)
}

/// x in the walker's own frame: the dot products with the rows of its matrix (F-33765), merged.
fn vlocal(rows: &[Vec3; 3], x: Vec3) -> Vec3 {
    Vec3::new(vdot3(rows[0], x), vdot3(rows[1], x), vdot3(rows[2], x))
}

/// The 3D length the slide start writes, sqrt(fma(z, z, fma(x, x, y y))).
fn len3(v: Vec3) -> f32 {
    use crate::physics::exact::fma;
    fma(v.z, v.z, fma(v.x, v.x, v.y * v.y)).sqrt()
}

/// The dodge's constants (the image's words).
const DODGE_DY: f32 = 5.0; // 0x828308BC
const DODGE_MOVING: u32 = 0x38D1_B717; // 1e-4, 0x82080750
const DODGE_DIAG: u32 = 0x3F35_04F3; // 0.70710677, 0x82007DE8
const DODGE_ACC_MAX: f32 = 10000.0; // 0x828308B4
const DODGE_ACC_SPEED: f32 = 100.0; // 0x828308B8
const DODGE_STEP: f32 = 0.25; // s, 0x8283086C
const DODGE_ACC_STEP: f32 = 0.25; // q, 0x82830870
/// K (0x8294A544): zero in the image, never written, read 0 in two running originals
const DODGE_K: f32 = 0.0;
const DODGE_T_MIN: u32 = 0x3586_37BD; // 1e-6, 0x82003774
const DODGE_DIR_EPS: u32 = 0x2B8C_BCCC; // 1e-12, 0x827D8520
const DODGE_FLIP: f32 = 3.0; // 0x820077F8
const DODGE_LIFT: u32 = 0x3E99_999A; // 0.3, 0x8283087C
const DODGE_INSIDE_R: u32 = 0x3D4C_CCCD; // 0.05 (x 2.0), 0x82830880
const DODGE_DEG: u32 = 0x3C8E_FA35; // pi / 180, 0x82006BFC
const DODGE_REACH: f32 = 10.0; // 0x82003A8C, also the lane limit's radius
const DODGE_LANE_GAP: f32 = 3.0; // 0x82830884
const DODGE_LANE_EPS: u32 = 0x3727_C5AC; // 1e-5, 0x82007A3C
const DODGE_TURNS: [f32; 3] = [0.0, -45.0, 45.0]; // 0x820DA334
/// The speech thresholds on the car's squared speed (0x8294A558 / 0x8294A55C, copied from 0x820DA32C / 0x820DA330).
const DODGE_SPEECH_FAST: u32 = 0x4453_0C5B; // 844.193
const DODGE_SPEECH_SLOW: u32 = 0x4374_C419; // 244.766
/// F-88694's mask, the type flag of what the walker stops inside / dodges into partly (the probes' query: type 640, mask
/// *(0x82830888) & ~0x20 = 0x381E; the ground probe's type 512).
pub(crate) const DODGE_INSIDE_MASK: u32 = 0x800;

/// A grid object as the threat test reads it (mcObstacle, F-15352).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DodgeCar {
    /// +6: 0 / 1 racers, 2 an ambient car (any other kind is passed over)
    pub kind: u8,
    /// kind 2: F-96472 (it rides a navsys lane rail), C+452 != 0 (a behaviour), F-17857 (its physics is active)
    pub rail: bool,
    pub beh: bool,
    pub active: bool,
    /// vt+4: the matrix's rows 0 and 2 and its place
    pub a: Vec3,
    pub c: Vec3,
    pub q: Vec3,
    /// vt+8 (kinds 0 / 1: F-98427) and +416 (kinds 0 / 1)
    pub vel: Vec3,
    pub prev: Vec3,
    /// vt+12 (width, length) and vt+28
    pub size: (f32, f32),
    pub rv: f32,
    /// ((+48)->+8)+580, the car's burnout charge (None: no car at +48)
    pub burn: Option<f32>,
    /// vt+32 != 0: the car's physics instance (left out of the probes)
    pub inst: bool,
}

/// The walker as the dodge reads it: P (its place), r (I+492), U (I+4768), the rows of its matrix (F-33765), bSex.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DodgeWalker {
    pub p: Vec3,
    pub r: f32,
    pub u: Vec3,
    pub rows: [Vec3; 3],
    pub female: bool,
}

/// The walker's avoidance part (M) as far as the dodge writes it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Avoid {
    /// +0 .. +64: the path points of the last object tested
    pub pts: [Vec3; 5],
    /// +80: the object dodged (index into the threat list)
    pub car: Option<usize>,
    /// +96: where the walker stopped inside a 0x800 object
    pub stop_at: Vec3,
    /// +128: the slide's flat direction
    pub dir: Vec3,
    /// +160 / +176: the slide's ends
    pub from: Vec3,
    pub to: Vec3,
    /// +236 the time to the car, +240 the slide's time, +244 its length
    pub t: f32,
    pub time: f32,
    pub len: f32,
    /// +248: the avoid list entry playing
    pub clip: Option<usize>,
    /// +252 / +256: the clip's yaw and length corrections
    pub yaw: f32,
    pub stretch: f32,
    /// +260: 0x2 a clip of its own playing, 0x4 sliding
    pub flags: u32,
}

/// The world as the dodge asks it.
pub(crate) trait DodgeWorld {
    /// F-88694's sphere (F-66549, type 640, mask 0x800, the walker left out): anything there.
    fn inside(&mut self, c: Vec3, r: f32) -> bool;
    /// F-27454: a ball of radius `r` swept from `a` to `b` (type 640, mask 0x381E; the walker and, with `car`, the
    /// car left out): where it hits (Q+128) and the type flags of what (archetype +16).
    fn sweep(&mut self, a: Vec3, b: Vec3, r: f32, car: bool) -> Option<(Vec3, u32)>;
    /// F-57005: the segment `a` -> `b` (type 512, mask 0x381E) hits something.
    fn ground(&mut self, a: Vec3, b: Vec3) -> bool;
    /// F-59579: the nearest lane point within `r` of `x` and its squared distance.
    fn lane(&mut self, x: Vec3, r: f32) -> Option<(Vec3, f32)>;
    /// F-21043: the mover of clip `clip` at `t`.
    fn mover(&mut self, clip: usize, t: f32) -> Vec3;
    /// F-84843: the stop timer.
    fn stop(&mut self);
    /// F-39461 with `kind` (0: the voice's +14 sound, 1: its +10).
    fn speech(&mut self, kind: u32);
    /// F-99671 (the temporary rails dropped) and F-50795 (the path given up).
    fn drop_rails(&mut self);
}

/// F-17399: the point of a -> b nearest to w (t of the 3D projection, clamped to 0 .. 1 by fsel), with t (its out
/// argument; `closest_on_segment` without it).
fn seg_nearest(a: Vec3, b: Vec3, w: Vec3) -> (f32, Vec3) {
    use crate::physics::exact::fma;
    let d = vsub(b, a);
    let l2 = fma(d.x, d.x, fma(d.y, d.y, d.z * d.z));
    let t = if l2 == 0.0 {
        0.0
    } else {
        let e = vsub(w, a);
        let t = fma(e.x, d.x, fma(e.y, d.y, e.z * d.z)) / l2;
        let t = if t >= 0.0 { t } else { 0.0 };
        if t - 1.0 >= 0.0 { 1.0 } else { t }
    };
    (t, vmadd(d, t, a))
}

/// F-50861: the lane limit at `x` along `dir`: the nearest lane point within 10 m (F-59579), the flat way to it
/// normalised (F-41334, else `dir`); none when the lane is not ahead (cos < 1e-6), else how far along `dir` the walker
/// may go to stay 3 m + its radius from the lane.
fn lane_limit(w: &DodgeWalker, x: Vec3, dir: Vec3, world: &mut dyn DodgeWorld) -> Option<f32> {
    use crate::physics::exact::fma;
    let (near, d2) = world.lane(x, DODGE_REACH)?;
    let n = vsub(near, x);
    let n = crate::aiv::norm_or(Vec3::new(n.x, 0.0, n.z), f32::from_bits(DODGE_LANE_EPS), dir);
    let c = fma(dir.x, n.x, fma(dir.z, n.z, dir.y * n.y));
    if c < f32::from_bits(DODGE_T_MIN) {
        return None;
    }
    Some(((d2.sqrt() - DODGE_LANE_GAP) - w.r) / c)
}

/// The probe query of F-94813 (F-13499's fields as the search sets them): Q+208 the walker's place, Q+32 / Q+48 the
/// swept ball's ends, Q+240 the way probed, Q+256 the reach, Q+4 / Q+12 the car left out.
struct DodgeProbe {
    pq: Vec3,
    from: Vec3,
    to: Vec3,
    way: Vec3,
    reach: f32,
    car: bool,
}

/// F-78951: one probe of length `l`. Nothing swept: the clear way is the lane limit + l (or the reach), at least l,
/// and ground under the swept end; a hit: the clear way is the plan distance to the hit less the radius - a 0x800
/// object nearer than l gives a partial dodge into it -, bounded by the lane limit + l, at least l. On success the
/// displacement is the way x l.
#[allow(clippy::too_many_arguments)]
fn dodge_probe(w: &DodgeWalker, q: &DodgeProbe, l: f32, disp: &mut Vec3, t: &mut f32, clear: &mut f32, f800: &mut bool, world: &mut dyn DodgeWorld) -> bool {
    use crate::physics::exact::fma;
    match world.sweep(q.from, q.to, w.r, q.car) {
        None => {
            let x = vmadd(q.way, l, q.pq);
            *clear = match lane_limit(w, x, q.way, world) {
                Some(m) => m + l,
                None => q.reach,
            };
            if *clear < l {
                return false;
            }
            if !world.ground(Vec3::new(q.to.x, q.to.y + 1.0, q.to.z), Vec3::new(q.to.x, q.to.y - 1.0, q.to.z)) {
                return false;
            }
            *disp = vmul(q.way, Vec3::splat(l));
            true
        }
        Some((at, flags)) => {
            let h = vsub(at, q.pq);
            let dh = fma(h.z, h.z, h.x * h.x).sqrt();
            let c0 = dh - w.r;
            *clear = if -c0 >= 0.0 { 0.0 } else { c0 };
            if flags & DODGE_INSIDE_MASK != 0 {
                *f800 = true;
                if *clear < l {
                    let lo = f32::from_bits(DODGE_T_MIN);
                    let den = if lo - dh >= 0.0 { lo } else { dh };
                    *disp = vmul(h, Vec3::splat(*clear / den));
                    *t = (*t / l) * dh;
                    return true;
                }
            }
            if *clear < l {
                return false;
            }
            let x = vmadd(q.way, l, q.pq);
            if let Some(m) = lane_limit(w, x, q.way, world) {
                let m = m + l;
                let c = if *clear - m >= 0.0 { m } else { *clear };
                *clear = if -c >= 0.0 { 0.0 } else { c };
            }
            if *clear < l {
                return false;
            }
            *disp = vmul(q.way, Vec3::splat(l));
            true
        }
    }
}

/// F-94813: the way out. The away direction turned by 0, -45 and +45 degrees about +y, each probed over near / cos (the
/// ball swept from U (r + 0.3) above the walker out to the reach + 10 m along the turned way laid in the ground's plane,
/// (U x way) x U); for a hit on the path (not the body) the same three turns of the opposite direction over far / cos -
/// these keep the swept end of the first three's last. `disp` comes back as the displacement.
#[allow(clippy::too_many_arguments)]
pub(crate) fn dodge_search(w: &DodgeWalker, car: bool, disp: &mut Vec3, near: f32, far: f32, path: bool, t: &mut f32, clear: &mut f32, f800: &mut bool, world: &mut dyn DodgeWorld) -> bool {
    use crate::physics::exact::{fma, guest_cos, guest_sin};
    *f800 = false;
    let from = vmadd(w.u, w.r + f32::from_bits(DODGE_LIFT), w.p);
    let mut q = DodgeProbe { pq: w.p, from, to: Vec3::ZERO, way: Vec3::ZERO, reach: 0.0, car };
    let turn = |v: Vec3, th: f32| {
        let ang = th * f32::from_bits(DODGE_DEG);
        let (sn, cs) = (guest_sin(ang as f64) as f32, guest_cos(ang as f64) as f32);
        (Vec3::new(fma(v.z, sn, v.x * cs), v.y, fma(v.z, cs, -(v.x * sn))), cs)
    };
    let swz = |v: Vec3| (Vec3::new(v.y, v.z, v.x), Vec3::new(v.z, v.x, v.y));
    for th in DODGE_TURNS {
        let (way, cs) = turn(*disp, th);
        let l = near / cs;
        q.way = way;
        q.reach = l + DODGE_REACH;
        let (u_yzx, u_zxy) = swz(w.u);
        let (w_yzx, w_zxy) = swz(way);
        let c1 = -vsub(vmul(w_yzx, u_zxy), vmul(w_zxy, u_yzx));
        let (c_yzx, c_zxy) = swz(c1);
        let n = -vsub(vmul(u_yzx, c_zxy), vmul(u_zxy, c_yzx));
        let n2 = vdot3(n, n);
        let y0 = vrsqrt(n2);
        let y0 = if y0 == f32::INFINITY { 0.0 } else { y0 };
        let n = vmul(n, Vec3::splat(vnr(n2, y0)));
        q.to = vadd(vmul(n, Vec3::splat(q.reach)), q.from);
        if dodge_probe(w, &q, l, disp, t, clear, f800, world) {
            return true;
        }
    }
    if path {
        for th in DODGE_TURNS {
            let (way, cs) = turn(vneg(*disp), th);
            let l = far / cs;
            q.way = way;
            q.reach = l + DODGE_REACH;
            if dodge_probe(w, &q, l, disp, t, clear, f800, world) {
                return true;
            }
        }
    }
    false
}

/// F-83234: start the slide by `disp` (the car dodge: `car` its index and velocity, `v8` its place - the walker's; the
/// sidestep F-26402: no car, `sidestep`). With an evade clip playing and no clip of its own, not a sidestep, the clip is
/// re-aimed (M+252 / +256 from its mover at its time + T); else, not a sidestep, the evade clip is picked (F-37443: the
/// move's and the approach's angles in the walker's frame, T, the least and the most move). The slide lasts the clip
/// less 8 frames (or T); speech, only when not sliding yet, by the car's squared speed, then the clip's AudioType (a
/// random bit without a clip). Then the slide P -> P + disp starts.
#[allow(clippy::too_many_arguments)]
pub(crate) fn slide_start(m: &mut Avoid, w: &DodgeWalker, items: &[AvoidItem], durs: &[f32], car: Option<(usize, Vec3)>, disp: Vec3, v8: Vec3, t: f32, clear: f32, sidestep: bool, world: &mut dyn DodgeWorld, speech: &mut crate::vehicles::parked::Mwc, pick: &mut crate::vehicles::parked::Mwc) {
    use crate::physics::exact::{fma, fnms};
    use std::f32::consts::{PI, TAU};
    let back = Vec3::new(0.0, 0.0, -1.0);
    let re = m.clip.is_some() && m.flags & 2 == 0 && !sidestep;
    if re {
        let tr = items[m.clip.unwrap()].clip.map_or(Vec3::ZERO, |c| world.mover(c, m.time + t));
        let fd = crate::aiv::ang_xz(tr, back) - crate::aiv::ang_xz(vlocal(&w.rows, disp), back);
        let x = if -(fd - PI) >= 0.0 { fd } else { fd - TAU };
        m.yaw = if x + PI >= 0.0 { x } else { x + TAU };
        m.stretch = len3(disp) - len3(tr);
    } else if !sidestep {
        let ld = vlocal(&w.rows, disp);
        let lv = vlocal(&w.rows, v8);
        let mn = fma(ld.z, ld.z, ld.x * ld.x).sqrt();
        let a2 = crate::aiv::ang_xz(lv, back);
        let a1 = crate::aiv::ang_xz(ld, back);
        let got = evade_pick(items, w.female, a1, a2, t, mn, clear, |c, at| world.mover(c, at.unwrap_or(durs[c])), pick);
        if let Some((_, yaw)) = got {
            m.yaw = yaw;
        }
        m.clip = got.map(|g| g.0);
        m.stretch = 0.0;
    }
    let sp2 = car.map_or(0.0, |(_, v)| fma(v.z, v.z, fma(v.x, v.x, v.y * v.y)));
    let (fast, slow) = (f32::from_bits(DODGE_SPEECH_FAST), f32::from_bits(DODGE_SPEECH_SLOW));
    let quiet = m.flags & 4 != 0;
    let mut say = None;
    match m.clip.and_then(|e| items[e].clip.map(|c| (e, c))) {
        Some((e, c)) => {
            if !re {
                m.len = fnms(8.0, f32::from_bits(0x3D08_8889), durs[c]);
                if !quiet {
                    say = if sp2 > fast {
                        Some(0)
                    } else if sp2 < slow {
                        Some(1)
                    } else {
                        match items[e].audio {
                            2 => Some(1),
                            1 => Some(0),
                            _ => None,
                        }
                    };
                }
            }
        }
        None => {
            let lo = f32::from_bits(DODGE_T_MIN);
            m.len = if t - lo >= 0.0 { t } else { lo };
            if !quiet && !sidestep {
                say = Some(if sp2 > fast {
                    0
                } else if sp2 < slow {
                    1
                } else {
                    speech.next() & 1
                });
            }
        }
    }
    if let Some(k) = say {
        world.speech(k);
    }
    m.flags = (m.flags & !6) | 4;
    m.from = w.p;
    m.to = vadd(disp, w.p);
    m.dir = crate::aiv::norm_or(Vec3::new(disp.x, 0.0, disp.z), f32::from_bits(DODGE_LANE_EPS), Vec3::ZERO);
    m.t = t;
    if !re {
        m.time = 0.0;
    }
    m.car = car.map(|c| c.0);
    world.drop_rails();
}

/// F-15352: the car threat, every frame. Over the grid objects (`cars`, in the 3x3 cells' order): racers, and ambient
/// cars off the lane rails with a behaviour or active physics; within 5 m in height, moving (1e-4); the body from its
/// rear to its leading point along the axis it moves on (across for a diagonal), then 3 steps of 0.25 s with a racer's
/// acceleration; the first segment the walker's projection falls inside of (0 < t < 1) within the half extent + its
/// radius in plan is the hit. Inside a 0x800 object: stop. Else the way out (F-94813) and the slide (F-83234).
#[allow(clippy::too_many_arguments)]
pub(crate) fn car_dodge(m: &mut Avoid, w: &DodgeWalker, cars: &[DodgeCar], dt: f32, items: &[AvoidItem], durs: &[f32], world: &mut dyn DodgeWorld, speech: &mut crate::vehicles::parked::Mwc, pick: &mut crate::vehicles::parked::Mwc) {
    use crate::physics::exact::fma;
    let p = w.p;
    let inv = {
        let x = vftz(dt);
        let y = vftz(1.0 / x);
        let k = -vftz(vftz(y * x) - 1.0);
        if x == 0.0 { 0.0 } else { vftz(vftz(y * k) + y) }
    };
    for (j, o) in cars.iter().enumerate() {
        match o.kind {
            0 | 1 => {}
            2 if !o.rail && (o.beh || o.active) => {}
            _ => continue,
        }
        if vftz(o.q.y - p.y).abs() > DODGE_DY {
            continue;
        }
        let v = vflat(o.vel);
        let s2 = vdot3(v, v);
        if f32::from_bits(DODGE_MOVING) > s2 {
            continue;
        }
        let speed = if s2 == 0.0 { 0.0 } else { vftz(s2 * vnr(s2, vrsqrt(s2))) };
        let g = vftz(f32::from_bits(DODGE_DIAG) * speed);
        let va = vdot3(v, o.a);
        let vc = vftz(0.0 - vdot3(v, o.c));
        let (mut acc, mut sideways) = (Vec3::ZERO, false);
        if o.kind <= 1 {
            let a0 = vflat(vmul(vsub(o.vel, o.prev), Vec3::splat(inv)));
            let a2 = vdot3(a0, a0);
            acc = if a2 > DODGE_ACC_MAX || DODGE_ACC_SPEED > s2 { Vec3::ZERO } else { a0 };
            sideways = o.burn.is_some_and(|b| b > 0.0) && va.abs() > g;
        }
        let (hl, hw) = (o.size.1 * 0.5, o.size.0 * 0.5);
        let ng = vftz(0.0 - g);
        let (p0, p1, r0) = if vc >= g {
            (vmadd(o.c, hl, o.q), vmadd(o.c, -hl, o.q), hw)
        } else if ng > vc {
            (vmadd(o.c, -hl, o.q), vmadd(o.c, hl, o.q), hw)
        } else if va >= g {
            (vmadd(o.a, -hw, o.q), vmadd(o.a, hw, o.q), hl)
        } else if ng > va {
            (vmadd(o.a, hw, o.q), vmadd(o.a, -hw, o.q), hl)
        } else {
            (vmadd(o.c, hl, o.q), vmadd(o.c, -hl, o.q), o.rv)
        };
        let aq = vmul(acc, Vec3::splat(DODGE_ACC_STEP));
        let h2 = (DODGE_ACC_STEP * DODGE_ACC_STEP) * 0.5;
        let p2 = vmadd(acc, h2, vmadd(v, DODGE_STEP, p1));
        let v1 = vadd(v, aq);
        let p3 = vmadd(acc, h2, vmadd(v1, DODGE_STEP, p2));
        let v2 = vadd(v1, aq);
        let v3 = vadd(v2, aq);
        let p4 = vmadd(acc, h2, vmadd(v2, DODGE_STEP, p3));
        m.pts = [p0, p1, p2, p3, p4];
        let reach = vftz(r0 + w.r);
        let reach2 = vftz(reach * reach);
        for k in 1..5usize {
            let (t, n) = seg_nearest(m.pts[k - 1], m.pts[k], p);
            if !(t > 0.0 && t < 1.0) {
                continue;
            }
            let off = vflat(vsub(n, p));
            let n2 = vdot3(off, off);
            if !(reach2 > n2) {
                continue;
            }
            // the hit: inside a 0x800 object stop (unless a clip of its own plays), nothing else
            if world.inside(Vec3::new(p.x, p.y + 1.0, p.z), fma(f32::from_bits(DODGE_INSIDE_R), 2.0, w.r)) {
                if m.flags & 2 == 0 {
                    world.stop();
                    m.stop_at = p;
                }
                return;
            }
            let tt = ((k as f32 - 2.0) + t) * DODGE_STEP;
            let lo = f32::from_bits(DODGE_T_MIN);
            let mut tt = if lo - tt >= 0.0 { lo } else { tt };
            let rr = vftz(vftz(DODGE_K + r0) + w.r);
            let dist = if n2 == 0.0 { 0.0 } else { vftz(n2 * vnr(n2, vrsqrt(n2))) };
            let (mut near, mut far) = (vftz(rr - dist), vftz(rr + dist));
            let mut off = vsub(p, n);
            if sideways {
                off = vmadd(v3, tt, off);
            }
            let d0 = vflat(off);
            let m2 = vdot3(d0, d0);
            let mut dir = if m2 > f32::from_bits(DODGE_DIR_EPS) { vmul(d0, Vec3::splat(vnr(m2, vrsqrt(m2)))) } else { Vec3::X };
            if m.flags & 4 != 0 && m.flags & 2 == 0 && 0.0 > vdot3(m.dir, dir) && -far < near + DODGE_FLIP {
                dir = vneg(dir);
                std::mem::swap(&mut near, &mut far);
            }
            let mut clear = 0.0;
            let mut f800 = false;
            if dodge_search(w, o.inst, &mut dir, near, far, k > 1, &mut tt, &mut clear, &mut f800, world) {
                slide_start(m, w, items, durs, Some((j, o.vel)), dir, vsub(o.q, p), tt, clear, false, world, speech, pick);
            }
            return;
        }
    }
}

/// The world the dodge asks, as the port has it (docs/reference/people/car_dodge_rule.md "Port differences"): the
/// engine's queries are not decoded, so `inside` is the city collider's sphere, `sweep` the city collider's sphere marched
/// from a to b in steps of r / 2 (what it touches first, as a 0x800 object; props, cars and other walkers are not in it),
/// `ground` the city collider's ray a -> b, `lane` the port's F-59579 (`vehicles::traffic::snap_to_lane`), `mover` the evade clip's
/// mover translation from its start (`Clip::mover`, the time clamped to the clip). What the dodge asks for is kept
/// and done by `dodge_frame`.
struct PedDodgeWorld<'a> {
    col: Option<&'a WorldCollider>,
    nav: Option<&'a NavSys>,
    evade: &'a [Clip],
    stop: bool,
    say: Option<u32>,
    dropped: bool,
}

impl DodgeWorld for PedDodgeWorld<'_> {
    fn inside(&mut self, c: Vec3, r: f32) -> bool {
        self.col.is_some_and(|col| col.sphere(c, r).is_some())
    }
    fn sweep(&mut self, a: Vec3, b: Vec3, r: f32, _car: bool) -> Option<(Vec3, u32)> {
        let col = self.col?;
        let d = b - a;
        let n = ((d.length() / (r * 0.5).max(0.05)).ceil() as usize).clamp(1, 400);
        (0..=n).find_map(|k| col.sphere(a + d * (k as f32 / n as f32), r).map(|(_, _, at)| (at, DODGE_INSIDE_MASK)))
    }
    fn ground(&mut self, a: Vec3, b: Vec3) -> bool {
        let Some(col) = self.col else { return true };
        let d = b - a;
        let l = d.length();
        l > 0.0 && col.raycast(a, d / l, l).is_some()
    }
    fn lane(&mut self, x: Vec3, r: f32) -> Option<(Vec3, f32)> {
        crate::vehicles::traffic::snap_to_lane(self.nav?, x, r).0.map(|h| (h.point, h.d2))
    }
    fn mover(&mut self, clip: usize, t: f32) -> Vec3 {
        let Some(c) = self.evade.get(clip) else { return Vec3::ZERO };
        let t = t.clamp(0.0, c.anim.duration.max(0.0));
        c.mover(c.frame(t, false)).0 - c.mover(0.0).0
    }
    fn stop(&mut self) {
        self.stop = true;
    }
    fn speech(&mut self, kind: u32) {
        self.say = Some(kind);
    }
    fn drop_rails(&mut self) {
        self.dropped = true;
    }
}

/// The dodge's two generators: the speech MWC (0x82860880) and the shared one the clip pick draws from (0x82860890;
/// in the original also drawn by others).
#[derive(Default)]
pub(crate) struct DodgeRng {
    speech: crate::vehicles::parked::Mwc,
    pick: crate::vehicles::parked::Mwc,
}

/// One frame of the car dodge for a walker (F-97113's call of F-15352, every frame, and the slide's consumer F-56160):
/// the threat test on the cars, then what it asked for - the stop timer, the speech, the rails and path dropped, the
/// evade clip picked (F-37443) started as a one-off clip -, and the slide P -> P + disp over M+244 (M+240 += dt, the
/// place the straight line's point at min(M+240 / M+244, 1)). With a clip the walker slides here and the clip plays
/// without its root motion; without one the port's sidestep slide (`Ped::slide`, walk or run by the slide's speed)
/// carries it. At the slide's end M+248 and bit 0x4 are cleared and the clip ends (the port's after-dodge: face the car,
/// angry). Returns true while a dodge's clip plays.
#[allow(clippy::too_many_arguments)]
fn dodge_frame(p: &mut Ped, ty: &PedType, threats: &[Threat], col: Option<&WorldCollider>, nav: Option<&NavSys>, dt: f32, rand: &mut dyn FnMut() -> f32, rng: &mut DodgeRng, evades: &mut u32, speech: &mut Vec<Speech>) -> bool {
    let m = &mut p.dodge;
    // M+260 bit 0x2: a clip of the walker's own (the port's angry and Pivot clips)
    m.flags = (m.flags & !2) | if matches!(p.state, State::Angry(_) | State::Turn(_)) { 2 } else { 0 };
    let w = DodgeWalker { p: p.pos, r: PED_RADIUS, u: p.ground.up, rows: [p.rot * Vec3::X, p.rot * Vec3::Y, p.rot * Vec3::Z], female: ty.female };
    let cars: Vec<DodgeCar> = threats.iter().map(|t| t.car).collect();
    let durs: Vec<f32> = ty.evade.iter().map(|c| c.anim.duration).collect();
    let mut world = PedDodgeWorld { col, nav, evade: &ty.evade, stop: false, say: None, dropped: false };
    let re = m.clip.is_some() && m.flags & 2 == 0;
    car_dodge(m, &w, &cars, dt, &ty.avoid, &durs, &mut world, &mut rng.speech, &mut rng.pick);
    if world.stop {
        p.stop = stop_time(ty.idle.anim.duration, rand);
        p.stops += 1;
    }
    if let Some(k) = world.say {
        speech.push(Speech { pos: p.pos, voice: ty.voice.clone(), female: ty.female, fast: k == 0 });
    }
    let m = p.dodge;
    let clip = m.clip.and_then(|e| ty.avoid.get(e)?.clip);
    if world.dropped {
        p.rail = None;
        p.path.clear();
        p.ride = None;
        p.dodged = m.car.and_then(|j| threats.get(j)).map(|t| t.pos);
        *evades += 1;
        match clip {
            // (re-aimed: the clip playing goes on)
            Some(c) if !(re && p.state == State::Evade(c)) => {
                start_one_off(p, State::Evade(c));
                p.slide = None;
            }
            Some(_) => p.slide = None,
            None => p.slide = Some((m.from, m.to, m.time, m.len)),
        }
    }
    if m.flags & 4 == 0 {
        return false;
    }
    // the slide (F-56160)
    let time = m.time + dt;
    p.dodge.time = time;
    let done = time >= m.len;
    if done {
        p.dodge.clip = None;
        p.dodge.flags &= !4;
    }
    if clip.is_none() || !matches!(p.state, State::Evade(_)) {
        // (no clip: the sidestep slide moves the walker)
        return false;
    }
    p.pos = m.from.lerp(m.to, (time / m.len).min(1.0));
    p.ground.t_face = flat(p.rot * -Vec3::Z).normalize_or(Vec3::Z);
    if done {
        // the clip ends with the slide: the caller's end of a one-off clip follows
        if let State::Evade(k) = p.state {
            p.time = p.time.max(ty.evade[k].anim.duration);
        }
    }
    true
}

/// Start locomotion clip k (0 idle, 1 walk, 2 run) at `rate` through F-52026, from t = 0: crossfaded in over 8 frames
/// (linear, `CLIP_BLEND_RATE`) from what plays, or straight in for the walker's first clip (the port's clips are all
/// resident: never parked).
fn start_loco(p: &mut Ped, k: i32, rate: f32) {
    let how = clip_start(&mut p.clips, k, rate, true);
    p.from = p.cur.clone();
    let b = if how == ClipStart::Blend { 1.0 / CLIP_BLEND_RATE } else { 0.0 };
    p.blend = b;
    p.blend_len = b.max(1e-6);
    p.time = 0.0;
    p.state = [State::Idle, State::Walk, State::Run][k as usize];
}

/// A one-off clip (dodge, angry, Pivot turn) started: F-20899 (not decoded beyond its slots: the one-off clip set, the
/// current and parked clips -1) - the port's 0.25 s blend.
fn start_one_off(p: &mut Ped, s: State) {
    p.from = p.cur.clone();
    p.blend = 0.25;
    p.blend_len = 0.25;
    p.time = 0.0;
    p.state = s;
    p.clips.cur = -1;
    p.clips.parked = -1;
    p.clips.oneoff = 1;
}

/// The state's clip unless it plays: idle is clip 0 at rate 1 (F-29547 state 0), a Pivot turn a one-off clip.
fn set_clip_state(p: &mut Ped, s: State) {
    match s {
        State::Idle if p.clips.cur != 0 => start_loco(p, 0, 1.0),
        State::Turn(_) | State::Evade(_) | State::Angry(_) if p.state != s => start_one_off(p, s),
        _ => {}
    }
}

/// F-29547 for the locomotion states 0 - 2 of a walker on the mesh: the clip `anim_choice` asks for (`start_loco`) or
/// the rate (F-66231); returns the clip's rate.
fn loco_clip(p: &mut Ped, ty: &PedType, st: u32, flags: u32, slide: f32) -> f32 {
    match anim_choice(st, p.clips.cur, p.clips.rate, 0, 0, flags, p.scale, || slide, &ty.a28, &ty.a36) {
        AnimCmd::Start(k, f) => start_loco(p, k, f),
        AnimCmd::Rate(f) => {
            p.clips.prev_rate = p.clips.rate;
            p.clips.rate = f;
        }
        _ => {}
    }
    p.clips.rate
}

/// One frame of a walker on the navigation mesh in the original's sidewalk behaviour
/// (mcAmbientBehaviorSidewalk::Update F-72378): stops (F-84843 / F-10083),
/// sidesteps (F-26402), rails round things in the way (F-79428), the path's
/// rails (F-61656, F-50894), car dodges (F-15352) and the turn round of a
/// reversed path (F-94431). Moves `p` and sets its state; returns the walk clip's
/// playback rate.
#[allow(clippy::too_many_arguments)]
fn sidewalk_step(p: &mut Ped, ty: &PedType, me: usize, peds: &[Vec3], props: &[Obst], col: Option<&WorldCollider>, nav: Option<&NavSys>, threats: &[Threat], due: u8, dt: f32, rand: &mut dyn FnMut() -> f32, rng: &mut DodgeRng, evades: &mut u32, speech: &mut Vec<Speech>) -> f32 {
    // (the behaviour's facing: the rail's, kept apart from the drawn one, which is the mean of the last 8 frames')
    let fwd = if p.ground.face != Vec3::ZERO { p.ground.face } else { flat(p.rot * -Vec3::Z).normalize_or(Vec3::Z) };
    p.ground.t_face = fwd;
    let set_state = set_clip_state;
    p.hold = (p.hold - dt).max(0.0);
    // the car dodge, every frame (F-15352 from F-97113, no gate of its own): a dodge's clip slides the walker here
    if dodge_frame(p, ty, threats, col, nav, dt, rand, rng, evades, speech) {
        return 1.0;
    }
    // clips playing (angry, turn): the caller moves the walker by their root
    if matches!(p.state, State::Evade(_) | State::Angry(_) | State::Turn(_)) {
        return 1.0;
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
        // (the target faces the push while sliding aside: F-94626 builds T's r2 from behaviour +400, the push F-83234
        // stores there)
        p.ground.t_face = flat(b - a).normalize_or(fwd);
        // (sliding, F-33058: walk or run by the slide's speed F-92375, which is the wanted speed)
        let s = slide_speed(a, b, len);
        let (st, want, lag) = locomotion(4, p.stop, true, p.scale, false, || s, &ty.a28, &ty.a36);
        p.speed += (want - p.speed) * (dt / lag).min(1.0);
        p.slide = if t >= len { None } else { Some((a, b, t, len)) };
        return loco_clip(p, ty, st, 4, s);
    }
    // a reversed path: turn round as it is taken (F-95960; a Pivot clip beyond
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
    // something ahead (F-10083, run every frame the walker is not waiting, on a detour
    // rail too): a rail round it, or stop. The candidates (F-92334): the corner of the
    // box furthest left as seen from the walker, pushed out to the left by half the walker's
    // width and 0.25 (0.55); for furniture also the corner furthest right pushed right; a
    // walker in the way is only ever passed on the left (F-10083 clears args +44 bit 0x40
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
    // along the rails, at the walking speed (F-33058: the speed scale x the walk clip's speed)
    let mut step = {
        let (_, want, lag) = locomotion(0, 0.0, true, p.scale, false, || 0.0, &ty.a28, &ty.a36);
        p.speed += (want - p.speed) * (dt / lag).min(1.0);
        p.speed * dt
    };
    let mut heading = fwd;
    for _ in 0..3 {
        if p.rail.is_none() {
            // the next path rail (F-61656): an arc-line-arc rail to a segment's middle; none drops the path and the
            // walker stands until the next one
            let pts: Vec<Vec3> = p.path.iter().map(|q| q.0).collect();
            match next_rail(&pts, p.pos, heading, ty.turn_radius) {
                Some(r) => {
                    p.rail = Some((r.t, false));
                    p.ride = Some((r, 0.0));
                }
                None => {
                    p.path.clear();
                    break;
                }
            }
        }
        let (target, around) = p.rail.unwrap();
        if let (false, Some((r, ph))) = (around, p.ride) {
            // ridden by phase (the phase advance F-70353, the point F-56929, the heading its tangent F-28568); what is
            // left past its end goes on to the next rail (the follower's carry for walkers is not decoded: the port's)
            let ph2 = r.advance(ph, step);
            let at = r.point(ph2);
            p.pos = Vec3::new(at.x, p.pos.y, at.z);
            heading = flat(r.tangent(ph2)).normalize_or(heading);
            if ph2 >= 1.0 {
                step = (ph2 - 1.0) * r.len();
                p.rail = None;
                p.ride = None;
                continue;
            }
            p.ride = Some((r, ph2));
            break;
        }
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
    p.ground.face = heading;
    p.ground.t_face = heading;
    loco_clip(p, ty, 1, 0, 0.0)
}

/// The pedestrian spawn candidates around `focus` (F-47658 -> F-12133): the
/// sidewalk polygons a sphere of radius `r` touches, the first 128 in the order of the
/// level's chunks and their quadtrees, sorted (`NavMesh::sphere_query`), with their areas.
pub(crate) fn spawn_candidates(mesh: &NavMesh, focus: Vec3, r: f32) -> Vec<(u32, f32)> {
    let found = mesh.sphere_query(focus, r, 128, |_, q| q.sidewalk() && q.nv >= 3 && q.closest(focus.x, focus.z).distance(focus) <= r);
    found.into_iter().map(|i| (i, mesh.polys[i as usize].area)).collect()
}

/// A random point of a polygon as the original picks it (F-76960): a triangle of the
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
    /// the car as the dodge's threat test reads it
    pub(crate) car: DodgeCar,
}

impl Threat {
    /// A car as a grid object of the dodge (F-15352): the kinds as `brainturn::ObjectInfo::from_car` takes them (the
    /// player 0, racers 1, others 2), an ambient car on a lane rail passed over and off one active; the matrix's rows 0
    /// and 2 and its place; the size 2 x the tuning box's half extents (`body_half`; the original's from the bound box)
    /// and vt+28 = sqrt(fma(hx, hx, hz hz)) as the racer's own extent writer has it; +416 taken as the velocity (its
    /// writer is not found: no acceleration); the burnout charge `burn`.
    pub fn from_car(c: &crate::vehicles::CarInstance) -> Threat {
        use crate::brainturn::ObjectInfo;
        let kind = ObjectInfo::from_car(0, c).kind;
        let h = c.params().body_half;
        let (r, q) = (c.state.rot, c.state.pos);
        let car = DodgeCar {
            kind,
            rail: c.rail.is_some(),
            beh: false,
            active: c.rail.is_none(),
            a: r * Vec3::X,
            c: r * Vec3::Z,
            q,
            vel: c.state.vel,
            prev: c.state.vel,
            size: (h.x * 2.0, h.z * 2.0),
            rv: crate::physics::exact::fma(h.x, h.x, h.z * h.z).sqrt(),
            burn: c.state.veh.as_ref().map(|v| v.burn),
            inst: true,
        };
        Threat { pos: q, vel: c.state.vel, car }
    }
}

pub struct Speech {
    pub pos: Vec3,
    pub voice: String,
    pub female: bool,
    pub fast: bool,
}

/// `tune/ambients/density_tuning.xml` (the original's density manager, vtable 0x820c39c4).
struct Tuning {
    /// within this a spawn point must not be in view (F-95989)
    spawn_inner: f32,
    /// radius of the sphere whose sidewalk polygons are spawn candidates
    spawn_outer: f32,
    /// unspawned further than this ahead, half of it behind (F-39594)
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
    /// the ambient manager's front-end mode (F-95688, the start screen): the spawner has no
    /// visibility test (F-33044, A-29552); the focus and view are the camera's
    pub frontend: bool,
    /// the traffic region list's count (traffic.rs `Vehicles::traffic_regions`, M +1792; None: no traffic spawner):
    /// the unspawners' dispatcher F-75661 runs the people's unspawner F-39594 only while it is not 0
    pub traffic_regions: Option<usize>,
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
    /// the avoid list in file order (the manager's +0x64228 list): clip name, AvoidType, ApproachDir, AudioType
    avoid_list: Vec<(String, u32, u32, u32)>,
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
    dodge_rng: DodgeRng,
    voices: Option<HashMap<String, String>>,
    /// the speech banks' scan, started ahead on a thread (`scan_voices_ahead`)
    voices_scan: Option<std::thread::JoinHandle<HashMap<String, String>>>,
    buf: SkinBuffer,
    /// at night: the lights the walkers' picker lays over their grid lamps, and the 30 Hz frames
    /// since the last frame (main.rs sets them each frame; None by day - figlight.rs)
    pub night_lights: Option<(crate::figlight::Dynamic, f32)>,
    /// the scene ambient (mgr + 224) the walkers' ambient eases towards with the fill map's colour (main.rs)
    pub amb_base: Vec3,
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
/// holds them next to the car types (F-54839).
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
            traffic_regions: None,
            districts: Distribution::default(),
            hour: 12.0,
            dirs: Vec::new(),
            tune_dir: PathBuf::new(),
            types: HashMap::new(),
            reading: HashMap::new(),
            ahead_also: None,
            avoid: Vec::new(),
            avoid_list: Vec::new(),
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
            dodge_rng: DodgeRng::default(),
            voices: None,
            voices_scan: None,
            buf: SkinBuffer::default(),
            night_lights: None,
            amb_base: Vec3::ZERO,
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
                const TYPES: [&str; 6] = ["AvoidAny", "AvoidMale", "AvoidFemale", "Pivot", "AngryMale", "AngryFemale"];
                const DIRS: [&str; 5] = ["Any", "Front", "Rear", "Left", "Right"];
                let ty = TYPES.iter().position(|t| *t == field("AvoidType")).unwrap_or(0) as u32;
                let dir = DIRS.iter().position(|t| *t == field("ApproachDir")).unwrap_or(0) as u32;
                const AUDIO: [&str; 3] = ["None", "Fast", "Slow"];
                let audio = AUDIO.iter().position(|t| *t == field("AudioType")).unwrap_or(0) as u32;
                self.avoid_list.push((name.clone(), ty, dir, audio));
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
        let run = Clip::bind(pick("run").or(pick("walk"))?, &model.ids, false, false);
        let a28 = [idle.anim.duration, walk.anim.duration, run.anim.duration];
        let a36 = [idle.travel.length(), walk.travel.length(), run.travel.length()];
        // "ped_fh_006_set" -> female, voice "fh_006"
        let core = name.trim_start_matches("ped_").trim_end_matches("_set").to_string();
        // (the type's bSex, +109: true for every "f" set, false for every "m" one)
        let female = std::fs::read_to_string(self.tune_dir.join(name).join("mcAmbientType.xml")).ok().and_then(|t| xml_bool(&t, "bSex")).unwrap_or_else(|| core.trim_start_matches("sc_").starts_with('f'));
        let gender = if female { "fped" } else { "mped" };
        let mut evade = Vec::new();
        let mut evade_names = Vec::new();
        let mut angry = Vec::new();
        // Pivot clips are chosen without regard to sex (F-16288)
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
                0 => {
                    evade.push(Clip::bind(a, &model.ids, *fast, *front));
                    evade_names.push(a.short());
                }
                1 => angry.push(Clip::bind(a, &model.ids, *fast, *front)),
                _ => {}
            }
        }
        let mut lod_switch = [40.0, 80.0];
        if let Ok(t) = std::fs::read_to_string(self.tune_dir.join(name).join("mcAmbientRenderType.xml")) {
            let s = xml_value(&t, "fLodSwitchScale").unwrap_or(1.0);
            lod_switch = [xml_value(&t, "fHighToMedSwitch").unwrap_or(40.0) * s, xml_value(&t, "fMedToLowSwitch").unwrap_or(80.0) * s];
        }
        let turn_radius = std::fs::read_to_string(self.tune_dir.join(name).join("mcAmbientType.xml")).ok().and_then(|t| xml_value(&t, "fMinTurningRadius")).unwrap_or(0.1);
        // the avoid list as the pick reads it: an entry's clip only when it is one of this sex's evades (the other
        // sex's entries and the angry / pivot ones fail the pick's type test anyway)
        let avoid = self.avoid_list.iter().map(|(n, ty, dir, audio)| AvoidItem { ty: *ty, dir: *dir, clip: evade_names.iter().position(|e| e == n), audio: *audio }).collect();
        Some(PedType { female, voice: core, model, walk, idle, run, a28, a36, evade, avoid, angry, pivot, lod_switch, turn_radius })
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

    /// The walkers' foot positions: each one's collision capsule (radius 0.3, 0.3 to 1.5 m above the feet) is in the
    /// level the police's line probe meets (F-20855).
    pub fn capsule_feet(&self) -> Vec<Vec3> {
        self.list.iter().map(|q| q.pos).collect()
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

    /// Within the view cone near the player (F-95989, pedestrian branch): closer than
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

    /// Outside the band the pedestrians live in (F-39594): more than half the box width
    /// to the side of the view line, or further than the unspawn radius ahead (half of it
    /// behind).
    fn out_of_range(&self, focus: Vec3, pos: Vec3) -> bool {
        let v = pos - focus;
        let dir = self.view_dir;
        let lateral = (v.z * dir.x - v.x * dir.z).abs();
        let lim = if v.dot(dir) < 0.0 { self.tune.unspawn * 0.5 } else { self.tune.unspawn };
        lateral > self.tune.box_width * 0.5 || flat(v).length() > lim
    }

    /// One run of the original's pedestrian spawner (F-33044) and unspawner
    /// (F-39594, from F-75661) on the navigation mesh: every other 30 Hz frame.
    fn spawn_frame(&mut self, gpu: &mut Gpu, mesh: &NavMesh, col: &WorldCollider, focus: Vec3) {
        // the type weights' use of the types (type +8, F-68379 -> F-83932): the walkers of each set;
        // a person's type tune has no bIsTrailer (the type's +112 there was not traced: taken as 0)
        let mut live: HashMap<String, i32> = HashMap::new();
        for p in &self.list {
            if let Some(d) = self.dirs.get(p.ty) {
                *live.entry(d.0.to_ascii_lowercase()).or_insert(0) += 1;
            }
        }
        self.districts.set_use(&|n| live.get(n).copied().unwrap_or(0), &|_| false);
        // unspawn: 30 random checks a frame, while the traffic's region list has roads (F-75661)
        for _ in 0..if self.traffic_regions == Some(0) { 0 } else { 30 } {
            if self.list.is_empty() {
                break;
            }
            let i = (self.rand() * self.list.len() as f32) as usize % self.list.len();
            // (an item's person goes like any walker, F-39594: no item test)
            if self.out_of_range(focus, self.list[i].pos) {
                let p = self.list.swap_remove(i);
                if std::env::var("MCLA_PED_DUMP").is_ok() {
                    let v = p.pos - focus;
                    log::info!("ped unspawn {} dist {:.1} ahead {:.1} lateral {:.1}", p.key.1, flat(v).length(), v.dot(self.view_dir), (v.z * self.view_dir.x - v.x * self.view_dir.z).abs());
                }
            }
        }
        // candidates: the sidewalk polygons touching the sphere of the outer spawn radius,
        // the first 128 (F-47658)
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
            // the walking speed factor, drawn once (F-57015)
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
                dodge: Avoid::default(),
                hold: 0.5,
                item: None,
                age: 0.0,
                life: 8.0,
                blend_len: 0.25,
                ground: Ground::default(),
                ride: None,
                clips: ClipSlots::default(),
            });
            spawned += 1;
        }
    }

    /// The frame's one path request (F-73065 lets one through per frame, to a walker
    /// without a path - it keeps one until it is cancelled - whose last request is at least
    /// 16 frames old): a random walk from where it stands, the way it faces (backwards after
    /// an even number of stops in a row, more than one), with the points on the edges `frac`
    /// (0.3 .. 0.7, per path) along them (F-44435).
    fn plan_frame(&mut self, mesh: &NavMesh) {
        for p in self.list.iter_mut() {
            p.since_plan = p.since_plan.saturating_add(1);
        }
        let n = self.list.len();
        for k in 0..n {
            let i = (self.plan_cursor + k) % n;
            let p = &self.list[i];
            if p.poly == NO_POLY || p.hold > 0.0 || p.since_plan < 16 || !p.path.is_empty() || !matches!(p.state, State::Walk | State::Idle | State::Run) {
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
                State::Run => "run",
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
                dodge: Avoid::default(),
                    hold: 0.0,
                    item: None,
                    age: 0.0,
                    life: 8.0,
                    blend_len: 0.25,
                    ground: Ground::default(),
                    ride: None,
                    clips: ClipSlots::default(),
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
                // (an item's person, F-39915: flag 0x08 clear - no walking, no avoiding;
                // no rail - locomotion state 0, the set's clip 0 (`*Ped_Idle*`) looped from 0
                // at rate 1, F-33058 / F-29547. The car check F-15352 runs all
                // the same: a dodge, then clip 0 again crossfaded in over 8 frames, F-52026)
                p.age += dt;
                p.blend = (p.blend - dt).max(0.0);
                // (its dodge: the same F-15352; a clip slides it, `dodge_frame`)
                if p.state == State::Idle || matches!(p.state, State::Evade(_)) {
                    dodge_frame(p, ty, threats, Some(col), Some(nav), dt, &mut rand, &mut self.dodge_rng, &mut self.evades, &mut self.speech);
                }
                let t0 = p.time;
                p.time += dt;
                if let State::Evade(k) = p.state {
                    // (no root motion: the dodge's slide moves it)
                    let clip = &ty.evade[k];
                    let _ = t0;
                    if p.time >= clip.anim.duration.max(0.05) {
                        p.from = p.cur.clone();
                        // (clip 0 again through F-52026: after a one-off clip a crossfade of 8 frames)
                        p.blend = 1.0 / CLIP_BLEND_RATE;
                        p.blend_len = 1.0 / CLIP_BLEND_RATE;
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
            // (where the walker is drawn before this update: F-49074's `cur`)
            let cur = p.pos;
            let mut due = 0u8;
            let frames = self.frame_no.wrapping_sub(frame0);
            if on_mesh {
                boxes.clear();
                obstacles(p.pos, 4.0, &mut boxes);
                // the spheres due in the 30 Hz frames run now: the one the counter points at, counter % 3, or
                // counter & 1 while a one-off clip plays (F-49074)
                let d = p.pos.distance(center);
                let chance = AWARE_CHANCE.iter().find(|c| d < c.0).map_or(AWARE_CHANCE_FAR, |c| c.1);
                let clip = matches!(p.state, State::Evade(_) | State::Angry(_) | State::Turn(_));
                let mut n = frame0;
                while n != self.frame_no {
                    n = n.wrapping_add(1);
                    if rand() < chance {
                        due |= 1 << (if clip { n & 1 } else { n % 3 });
                    }
                }
                rate = sidewalk_step(p, ty, i, &peds_pos, &boxes, Some(col), Some(nav), threats, due, dt, &mut rand, &mut self.dodge_rng, &mut self.evades, &mut self.speech);
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
                // (a dodge's clip on the mesh: no root motion, the slide moves the walker - F-56160)
                if !(on_mesh && matches!(p.state, State::Evade(_))) {
                    apply(p, clip.frame(t0, false), clip.frame(p.time, false));
                }
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
                        // the angry clip, else wait (F-73407)
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
                    if on_mesh {
                        let s = p.state;
                        if s == State::Idle {
                            // (clip 0 after the one-off clip: F-52026's crossfade of 8 frames)
                            start_loco(p, 0, 1.0);
                        } else {
                            start_one_off(p, s);
                        }
                    }
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
            if let Some(m) = mesh {
                // the polygon walked on
                if !m.polys[p.poly as usize].contains(p.pos.x, p.pos.z) {
                    if let Some(k) = m.find(p.pos, !0, 2.0) {
                        p.poly = k;
                    }
                }
                // the ground, U and the drawn matrix (F-49074)
                walker_ground(p, col, cur, frames, due & 1 != 0, dt);
            } else if let Some(g) = col.ground(p.pos, 0.7, 1.5) {
                // (kerb walkers, the port's own: eased onto the collision)
                p.pos.y += (g.point.y - p.pos.y) * (dt * 12.0).min(1.0);
            }
        }
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223 + self.list.len() as u32);
    }

    /// F-39915: a layout item's person, standing at the item's place facing
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
                dodge: Avoid::default(),
            hold: 0.0,
            item: Some(item),
            age: 0.0,
            life,
            blend_len: 0.25,
            ground: Ground::default(),
            ride: None,
            clips: ClipSlots::default(),
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

    /// The walkers' LOD by the main view, as the original's walker render routine (F-68474, main
    /// pass) gets it from its vt+44 F-68463 -> F-69447(the set's render type, the sphere's
    /// centre, radius 0.9 (0x82007FA8), flag 0): -1 when the sphere is outside the view or behind an
    /// occluder or its far side is 500 or more in front of the near plane (0x82003250); LOD 0 below
    /// the set's `fHighToMedSwitch` x `fLodSwitchScale`, LOD 1 below `fMedToLowSwitch` x the scale,
    /// else LOD 2 (every set: 40 / 80). There is no other distance limit. OPTIONS > PERFORMANCE
    /// (`k`): DRAW DISTANCE x the 500, DETAIL LEVEL x the steps (3X: LOD 0 throughout).
    /// The centre: the original takes the walker's matrix x its bound's centroid (F-41581);
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

    /// Skin everything in view into this frame's vertex buffer. `hidden`: the cops' arrest scene is on screen (F-36139) -
    /// the walker render F-68474 returns before its light step and its draw, so nothing is emitted and the lights stay as
    /// they were.
    pub fn prepare(&mut self, gpu: &Gpu, cam: Vec3, frustum: &Frustum, hidden: bool) {
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
            if !visible || hidden {
                continue;
            }
            let lod = lod as usize;
            // (at night the four lights of its program, picked and eased as the original's
            // F-44222 does at the walker's place - only while it is drawn, as there)
            if let (Some((dynamic, frames)), Some(t)) = (&self.night_lights, &tables) {
                let f = crate::fillmap::body_now(p.pos.x, p.pos.z);
                let amb = self.amb_base + Vec3::new(f[0], f[1], f[2]);
                let fl = self.fig_lights.entry(p.key).or_insert_with(|| crate::figlight::FigureLight::new(t, p.pos, amb));
                fl.step(t, p.pos, *frames);
                fl.step_amb(amb, *frames);
                self.buf.next_lanes = Some(fl.lanes(p.pos, dynamic));
                // (MCLA_PED_AMB_OLD=1: the ambient as before, the scene's plus the fill map at once, unpacked)
                if std::env::var("MCLA_PED_AMB_OLD").is_err() {
                    self.buf.next_amb = Some(fl.amb_bytes());
                }
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

    fn ref_word(t: &str) -> u32 {
        u32::from_str_radix(&t.replace('_', ""), 16).unwrap()
    }

    /// The walker's slot 80 (F-49074) whole against the original's code run offline on 3,000 cases
    /// (tools/ped_ground_reference.py -> testdata/ped_ground_ref.txt, with F-24029, F-59837, F-89721 and F-22858 in it):
    /// the sphere draw (chance by the distance to the focus, the walker's MWC, k by the counter, clip or not) and the
    /// spheres it sets, the 12 hit slots of sphere 0 (empty, of other types, normals about y = 0.70710677, plan distances
    /// about the 0.25 m^2 tie in any order), U zero / a normal / any, the clip, hit and no-hit tails and the forward ring in
    /// every state. The draw is reproduced here from the decode (the port draws by its own chance loop); the pick, U,
    /// the place, the facing, the ring and the drawn rows are the port's. Every xyz word must be equal (w lanes are not
    /// the port's).
    #[test]
    fn ped_ground_matches_the_original() {
        let (mut n, mut hits, mut clips, mut slides) = (0, 0, 0, 0);
        for l in include_str!("testdata/ped_ground_ref.txt").lines() {
            let (c, o) = l.split_once(" -> ").unwrap();
            let c: Vec<u32> = c.split(' ').map(ref_word).collect();
            let o: Vec<u32> = o.split(' ').map(ref_word).collect();
            let f = |i: usize| f32::from_bits(c[i]);
            let v = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            let ov = |i: usize| Vec3::new(f32::from_bits(o[i]), f32::from_bits(o[i + 1]), f32::from_bits(o[i + 2]));
            let bits = |v: Vec3| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()];
            let clip = c[0] != 0 && c[1] == 3;
            let (t_r2, t_place, cur) = (v(15), v(18), v(21));
            // the draw (F-49074 step 1, F-59837), as decoded
            let mut mwc = crate::vehicles::parked::Mwc { lo: c[5], carry: c[6] };
            let step = |m: &mut crate::vehicles::parked::Mwc| {
                let x = m.lo as u64 * 0x5CDC_FAA7 + m.carry as u64;
                m.lo = x as u32;
                m.carry = (x >> 32) as u32;
                (m.lo & 0x7F_FFFF) as f32
            };
            let p = if c[2] == 0 {
                0.4f32
            } else {
                let (dx, dz) = (f(27) - f(24), f(29) - f(26));
                let d = crate::physics::exact::fma(dz, dz, dx * dx);
                if d < 625.0 { 0.4 } else if d < 7225.0 { 0.15 } else { 0.1 }
            };
            let mut spheres = [(0u32, 0u32, [0u32; 3]); 3];
            let mut centre = v(33);
            if step(&mut mwc) * f32::from_bits(0x3400_0000) < p {
                // (no manager: the counter read is the empty page's 0)
                let cnt = if c[2] != 0 { c[4] } else { 0 };
                let k = if clip { cnt & 1 } else { cnt % 3 } as usize;
                let r = if k == 0 { GROUND_RADIUS } else { f32::from_bits(0x3F2A_C083) };
                let mut at = t_place;
                if k == 2 {
                    let s = step(&mut mwc) * f32::from_bits(0x34A0_0000);
                    at = crate::tcp_rail::vmadd3(vneg(t_r2), s, t_place);
                }
                if k == 0 {
                    centre = at;
                }
                spheres[k] = (1, r.to_bits(), bits(at));
            }
            // the pick (F-24029) and the tail
            let slots: Vec<Option<GroundContact>> = (0..12)
                .map(|j| {
                    let s = 66 + 8 * j;
                    (c[s] != 0).then(|| GroundContact { kind: c[s + 1], point: v(s + 2), normal: v(s + 5) })
                })
                .collect();
            let pick = ground_pick(&slots, centre);
            let mut up = v(30);
            let (place, r2) = ground_place(t_r2, t_place, cur, &mut up, pick, f(7), f(8), clip);
            let mut ring = FwdRing { count: c[36] as i32, index: c[37] as i32, sum: v(38), e: [Vec3::ZERO; 8] };
            for (k, e) in ring.e.iter_mut().enumerate() {
                *e = v(41 + 3 * k);
            }
            if c[65] != 0 {
                ring.reset(r2);
            } else {
                ring.push(r2);
            }
            let (r0, r2d) = ring.rows();
            let got = (bits(r0), bits(Vec3::Y), bits(r2d), bits(place), bits(up), ring.count as u32, ring.index as u32, bits(ring.sum));
            let want = (bits(ov(1)), bits(ov(5)), bits(ov(9)), bits(ov(13)), bits(ov(17)), o[21], o[22], bits(ov(23)));
            assert_eq!(got, want, "{l}");
            for k in 0..8 {
                assert_eq!(bits(ring.e[k]), bits(ov(27 + 4 * k)), "{l}");
            }
            for (k, s) in spheres.iter().enumerate() {
                let b = 59 + 5 * k;
                if s.0 != 0 {
                    assert_eq!((s.0, s.1, s.2), (o[b], o[b + 1], [o[b + 2], o[b + 3], o[b + 4]]), "{l}");
                } else {
                    assert_eq!(o[b], 0, "{l}");
                }
            }
            assert_eq!((mwc.lo, mwc.carry), (o[74], o[75]), "{l}");
            n += 1;
            hits += pick.is_some() as usize;
            clips += clip as usize;
            slides += (!clip && pick.is_none()) as usize;
        }
        assert_eq!(n, 3000);
        // (1,932 with a ground hit, 1,065 in a clip, 690 sliding without a hit)
        assert_eq!((hits, clips, slides), (1932, 1065, 690));
    }

    /// The forward ring alone (F-22858 push, F-89721 reset) against the original's code run offline: 600 sequences of up
    /// to 24 pushes / resets from an empty ring (testdata/ped_ring_ref.txt), the ring after every step equal bit for bit;
    /// and what it does to the drawn heading: a turn reaches the body over 8 frames, a one-off clip shows at once.
    #[test]
    fn forward_ring_matches_the_original() {
        let mut steps = 0;
        for l in include_str!("testdata/ped_ring_ref.txt").lines() {
            let (c, o) = l.split_once(" -> ").unwrap();
            let c: Vec<u32> = c.split(' ').map(ref_word).collect();
            let mut ring = FwdRing::default();
            for (i, after) in o.split(" | ").enumerate() {
                let a: Vec<u32> = after.split(' ').map(ref_word).collect();
                let v = Vec3::new(f32::from_bits(c[4 * i + 1]), f32::from_bits(c[4 * i + 2]), f32::from_bits(c[4 * i + 3]));
                if c[4 * i] == 2 {
                    ring.reset(v);
                } else {
                    ring.push(v);
                }
                let mut got = vec![ring.count as u32, ring.index as u32];
                for w in std::iter::once(ring.sum).chain(ring.e) {
                    got.extend([w.x.to_bits(), w.y.to_bits(), w.z.to_bits(), a[got.len() + 3]]);
                }
                assert_eq!(got, a, "{l}");
                steps += 1;
            }
        }
        assert_eq!(steps, 7319);
        // a quarter turn: the drawn facing moves an eighth of the way per frame and is there after 8 frames
        let mut ring = FwdRing::default();
        ring.reset(Vec3::Z);
        let mut yaw = Vec::new();
        for _ in 0..8 {
            ring.push(Vec3::X);
            let (_, r2) = ring.rows();
            yaw.push(r2.x.atan2(r2.z));
        }
        assert!(yaw.windows(2).all(|w| w[1] > w[0]) && (yaw[7] - std::f32::consts::FRAC_PI_2).abs() < 1e-6 && yaw[0] < 0.2, "{yaw:?}");
        ring.reset(Vec3::NEG_X);
        assert_eq!(ring.rows().1, Vec3::NEG_X);
    }

    /// The evade clip pick (F-37443 with the xz angle F-32060) against the original's code run offline on 4,000 cases
    /// (tools/evade_pick_reference.py -> testdata/evade_pick_ref.txt): lists of up to 10 entries of every AvoidType and
    /// ApproachDir, clips present or not, the threat angles at the sector edges a float step either side, the clips'
    /// moves about 50 degrees off the threat angle and about the minimum and maximum distance, the shared MWC's state.
    /// The entry, its angle and the MWC after must be equal bit for bit.
    #[test]
    fn evade_pick_matches_the_original() {
        let h = |t: &str| u32::from_str_radix(t, 16).unwrap();
        let f = |t: &str| f32::from_bits(h(t));
        let (mut n, mut picked) = (0, 0);
        for l in include_str!("testdata/evade_pick_ref.txt").lines() {
            let (c, o) = l.split_once(" -> ").unwrap();
            let c: Vec<&str> = c.split(' ').collect();
            let o: Vec<&str> = o.split(' ').collect();
            let k = |i: usize, j: usize| 9 + 9 * i + j;
            let items: Vec<AvoidItem> = (0..h(c[0]) as usize).map(|i| AvoidItem { ty: h(c[k(i, 0)]), dir: h(c[k(i, 1)]), clip: (h(c[k(i, 2)]) != 0).then_some(i), audio: 0 }).collect();
            let v = |i: usize, j: usize| Vec3::new(f(c[k(i, j)]), f(c[k(i, j + 1)]), f(c[k(i, j + 2)]));
            let d = |t: &str| h(&t.replace('_', ""));
            let mut rng = crate::vehicles::parked::Mwc { lo: d(c[7]), carry: d(c[8]) };
            let got = evade_pick(&items, h(c[1]) != 0, f(c[2]), f(c[3]), f(c[4]), f(c[5]), f(c[6]), |i, t| if t.is_some() { v(i, 3) } else { v(i, 6) }, &mut rng);
            let (e, a) = got.map(|(e, a)| (e as i32, a.to_bits())).unwrap_or((-1, u32::MAX));
            assert_eq!((e, a, rng.lo, rng.carry), (o[0].parse::<i32>().unwrap(), h(o[1]), d(o[2]), d(o[3])), "{l}");
            n += 1;
            picked += (e >= 0) as usize;
        }
        assert_eq!((n, picked), (4000, 566));
    }

    /// The world of a car dodge reference case: the stubs' answers in order, every call logged as the harness does.
    struct DodgeCase<'a> {
        c: &'a [u32],
        log: Vec<String>,
        sw: usize,
        gr: usize,
        ln: usize,
        mc: [u32; 8],
    }

    // the case's words (tools/car_dodge_reference.py)
    const DC_SW: usize = 43;
    const DC_GR: usize = 73;
    const DC_LN: usize = 79;
    const DC_ENT: usize = 110;
    const DC_OBJ: usize = 178;
    const DC_X: usize = 308;

    impl DodgeCase<'_> {
        fn w(&mut self, t: u32, ws: &[u32]) {
            self.log.push(format!("t{t}"));
            self.log.extend(ws.iter().map(|w| format!("{w:x}")));
        }
        fn v(&self, i: usize) -> Vec3 {
            Vec3::new(f32::from_bits(self.c[i]), f32::from_bits(self.c[i + 1]), f32::from_bits(self.c[i + 2]))
        }
    }

    impl DodgeWorld for DodgeCase<'_> {
        fn inside(&mut self, c: Vec3, r: f32) -> bool {
            self.w(1, &[c.x.to_bits(), c.y.to_bits(), c.z.to_bits(), r.to_bits()]);
            self.c[42] != 0
        }
        fn sweep(&mut self, a: Vec3, b: Vec3, r: f32, car: bool) -> Option<(Vec3, u32)> {
            self.w(3, &[a.x.to_bits(), a.y.to_bits(), a.z.to_bits(), b.x.to_bits(), b.y.to_bits(), b.z.to_bits(), r.to_bits(), car as u32]);
            let s = DC_SW + 5 * self.sw;
            self.sw += 1;
            (self.c[s] != 0).then(|| (self.v(s + 1), self.c[s + 4]))
        }
        fn ground(&mut self, a: Vec3, b: Vec3) -> bool {
            self.w(4, &[a.x.to_bits(), a.y.to_bits(), a.z.to_bits(), b.x.to_bits(), b.y.to_bits(), b.z.to_bits()]);
            self.gr += 1;
            self.c[DC_GR + self.gr - 1] != 0
        }
        fn lane(&mut self, x: Vec3, r: f32) -> Option<(Vec3, f32)> {
            self.w(5, &[x.x.to_bits(), x.y.to_bits(), x.z.to_bits(), r.to_bits()]);
            let s = DC_LN + 5 * self.ln;
            self.ln += 1;
            (self.c[s] != 0).then(|| (x + self.v(s + 1), f32::from_bits(self.c[s + 4])))
        }
        fn mover(&mut self, clip: usize, t: f32) -> Vec3 {
            self.w(6, &[clip as u32, t.to_bits()]);
            let k = if self.mc[clip] > 0 { 1 } else { 0 };
            self.mc[clip] += 1;
            self.v(DC_ENT + 11 * clip + 5 + 3 * k)
        }
        fn stop(&mut self) {
            self.w(2, &[]);
        }
        fn speech(&mut self, kind: u32) {
            self.w(7, &[kind]);
        }
        fn drop_rails(&mut self) {
            self.w(8, &[]);
            self.w(9, &[]);
        }
    }

    /// The walkers' car dodge against the original's code run offline (tools/car_dodge_reference.py ->
    /// testdata/car_dodge_ref.txt): the threat F-15352 whole on up to 5 grid objects of every kind (racers with and without
    /// a car, ambient cars on / off the rails, moving along / across / diagonally, accelerating or not, a burnout charge,
    /// in and out of the 5 m height band), with the direction search, probes, lane limit, the slide start and the evade
    /// pick inside it; the search F-94813 and the slide start F-83234 called on their own. The world's queries are the
    /// case's answers; every query (its words, in order), the avoidance part's words and the two MWCs must be equal.
    #[test]
    fn car_dodge_matches_the_original() {
        let h = |t: &str| u32::from_str_radix(&t.replace('_', ""), 16).unwrap();
        let mut n = [0usize; 3];
        for l in include_str!("testdata/car_dodge_ref.txt").lines() {
            let (ci, o) = l.split_once(" -> ").unwrap();
            let c: Vec<u32> = ci.split(' ').map(h).collect();
            let want: Vec<String> = o.split(' ').map(|t| if t.starts_with('t') { t.to_string() } else { format!("{:x}", h(t)) }).collect();
            let f = |i: usize| f32::from_bits(c[i]);
            let v = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            let w = DodgeWalker { p: v(1), r: f(4), u: v(5), rows: [v(8), v(11), v(14)], female: c[17] != 0 };
            let nent = c[109] as usize;
            let items: Vec<AvoidItem> = (0..nent)
                .map(|i| {
                    let e = DC_ENT + 11 * i;
                    AvoidItem { ty: c[e], dir: c[e + 1], audio: c[e + 2], clip: (c[e + 3] != 0).then_some(i) }
                })
                .collect();
            let durs: Vec<f32> = (0..6).map(|i| f(DC_ENT + 11 * i + 4)).collect();
            let cars: Vec<DodgeCar> = (0..c[176] as usize)
                .map(|j| {
                    let o = DC_OBJ + 26 * j;
                    let has = c[o + 23] != 0;
                    DodgeCar {
                        kind: c[o + 1] as u8,
                        rail: c[o + 2] != 0,
                        beh: c[o + 3] != 0,
                        active: c[o + 4] != 0,
                        a: v(o + 5),
                        c: v(o + 8),
                        q: v(o + 11),
                        vel: v(o + 14),
                        prev: v(o + 17),
                        size: (f(o + 20), f(o + 21)),
                        rv: f(o + 22),
                        burn: has.then(|| f(o + 24)),
                        inst: has && c[o + 25] != 0,
                    }
                })
                .collect();
            let mut m = Avoid {
                dir: v(19),
                clip: (c[22] != u32::MAX).then_some(c[22] as usize),
                time: f(23),
                len: f(24),
                yaw: f(25),
                stretch: f(26),
                t: f(27),
                stop_at: v(28),
                from: v(31),
                to: v(34),
                flags: c[18],
                ..Default::default()
            };
            let mut speech = crate::vehicles::parked::Mwc { lo: c[37], carry: c[38] };
            let mut pick = crate::vehicles::parked::Mwc { lo: c[39], carry: c[40] };
            let mut world = DodgeCase { c: &c, log: Vec::new(), sw: 0, gr: 0, ln: 0, mc: [0; 8] };
            let x = DC_X;
            let mut out: Vec<u32> = Vec::new();
            let vw = |o: &mut Vec<u32>, v: Vec3| o.extend([v.x.to_bits(), v.y.to_bits(), v.z.to_bits()]);
            if c[0] == 1 {
                let mut dir = v(x + 1);
                let (mut t, mut clear, mut f800) = (f(x + 7), f(x + 8), false);
                let r = dodge_search(&w, c[x] != 0, &mut dir, f(x + 4), f(x + 5), c[x + 6] != 0, &mut t, &mut clear, &mut f800, &mut world);
                out.push(r as u32);
                vw(&mut out, dir);
                out.extend([t.to_bits(), clear.to_bits(), f800 as u32]);
            } else {
                if c[0] == 0 {
                    car_dodge(&mut m, &w, &cars, f(41), &items, &durs, &mut world, &mut speech, &mut pick);
                } else {
                    let car = (c[x] != u32::MAX).then(|| (c[x] as usize, cars[c[x] as usize].vel));
                    slide_start(&mut m, &w, &items, &durs, car, v(x + 1), v(x + 4), f(x + 7), f(x + 8), c[x + 10] != 0, &mut world, &mut speech, &mut pick);
                }
                for p in m.pts {
                    vw(&mut out, p);
                }
                out.push(m.car.map_or(u32::MAX, |j| j as u32));
                vw(&mut out, m.stop_at);
                vw(&mut out, m.dir);
                vw(&mut out, m.from);
                vw(&mut out, m.to);
                out.extend([m.t.to_bits(), m.time.to_bits(), m.len.to_bits(), m.clip.map_or(u32::MAX, |e| e as u32)]);
                out.extend([m.yaw.to_bits(), m.stretch.to_bits(), m.flags, speech.lo, speech.carry, pick.lo, pick.carry]);
            }
            let mut got: Vec<String> = out.iter().map(|w| format!("{w:x}")).collect();
            got.extend(world.log);
            assert_eq!(got.join(" "), want.join(" "), "{l}");
            n[c[0] as usize] += 1;
        }
        assert_eq!(n, [3000, 500, 500]);
    }

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
    /// (tools/ped_walk_reference.py part d): the word it stores is TURN_RATE's bits.
    #[test]
    fn turn_rate_is_the_originals_static_init() {
        let l = include_str!("testdata/ped_walk_ref.txt").lines().find(|l| l.starts_with("d -> ")).unwrap();
        assert_eq!(ref_word(&l[5..]), TURN_RATE.to_bits());
    }

    fn walk_ref(tag: char) -> impl Iterator<Item = (Vec<u32>, &'static str)> {
        include_str!("testdata/ped_walk_ref.txt").lines().filter(move |l| l.starts_with(tag) && l.as_bytes()[1] == b' ').map(|l| {
            let (c, o) = l[2..].split_once(" -> ").unwrap();
            (c.split(' ').map(ref_word).collect(), o)
        })
    }

    /// The next path rail F-61656 (with F-93741, F-17399, F-16360, F-85722, F-65534, F-62306) and F-16360 alone against the
    /// original's code run offline on 3,000 paths (tools/ped_walk_reference.py part a: 0 - 48 points with zero, short and
    /// long steps, the walker on a point, on a segment's middle or about the path, unit / scaled / tilted / zero headings,
    /// radii 0.05 - 10 m, turn limits 1.2566371, 3.7699115, pi or any): the nearest segment, and every word of the rail
    /// record (A, B, c1, c2, P, T, H0, H1 xyz, a1, a2, R, the sense and valid bits) equal, or no valid rail on both sides.
    #[test]
    fn path_rail_matches_the_original() {
        let (mut n, mut valid) = (0, 0);
        let same = |o: &[u32], q: Option<crate::tcp_rail::TcpRail>| -> bool {
            let Some(q) = q else { return (o[40] >> 24) & 0x20 == 0 };
            let v = |i: usize, w: Vec3| o[i] == w.x.to_bits() && o[i + 1] == w.y.to_bits() && o[i + 2] == w.z.to_bits();
            v(0, q.a) && v(4, q.b) && o[8] == q.a1.to_bits() && o[9] == q.a2.to_bits() && v(12, q.c1) && v(16, q.c2) && o[20] == q.r.to_bits()
                && v(24, q.p) && v(28, q.t) && v(32, q.h0) && v(36, q.h1) && (o[40] >> 24) as u8 & 0xE0 == q.flags & 0xE0
        };
        for (c, o) in walk_ref('a') {
            let f = |i: usize| f32::from_bits(c[i]);
            let v = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            let pts: Vec<Vec3> = (0..c[0] as usize).map(|k| v(10 + 3 * k)).collect();
            let (p, h0, r) = (v(4), v(7), f(2));
            let (near, rest) = o.split_once(' ').unwrap();
            let near: i32 = near.parse().unwrap();
            assert_eq!(nearest_segment(&pts, p).map_or(-1, |i| i as i32), near, "case {n}");
            let (a, b) = rest.split_once(" | ").map_or((rest, None), |(a, b)| (a, Some(b)));
            let a: Vec<u32> = a.split(' ').map(ref_word).collect();
            let q = next_rail(&pts, p, h0, r);
            valid += q.is_some() as usize;
            assert!(same(&a, q), "case {n}: next rail {q:?}");
            if let Some(b) = b {
                let b: Vec<u32> = b.split(' ').map(ref_word).collect();
                let q = rail_to_mid(&pts, c[1] as usize, p, h0, r, f(3));
                assert!(same(&b, q), "case {n}: rail to the middle {q:?}");
            }
            n += 1;
        }
        assert_eq!(n, 3000);
        assert!(valid > 1000);
    }

    /// The locomotion state F-33058 (with F-71036 and the slide speed F-92375) against the original's code on 3,000 cases
    /// (part b: every flag combination and stop count, stop timers 0 / -0 / NaN / any, rail or none, clip or none, clip
    /// lengths 0 or not, slides of any length and time): the state, the wanted speed and its lag, every word equal.
    #[test]
    fn locomotion_matches_the_original() {
        let mut n = 0;
        for (c, o) in walk_ref('b') {
            let o: Vec<u32> = o.split(' ').map(ref_word).collect();
            let f = |i: usize| f32::from_bits(c[i]);
            let v = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            let (a28, a36) = ([f(5), f(6), f(7)], [f(8), f(9), f(10)]);
            let (st, want, lag) = locomotion(c[0], f(3), c[1] != 0, f(4), c[2] == 2, || slide_speed(v(11), v(14), f(17)), &a28, &a36);
            assert_eq!([st, want.to_bits(), lag.to_bits()], [o[0], o[1], o[2]], "case {n}");
            n += 1;
        }
        assert_eq!(n, 3000);
    }

    /// The clip start F-52026 against the original's code on 3,000 cases (part c: current / previous / parked clips,
    /// a one-off clip or none, resident or not, any rates): the six slots after it and the motion-tree calls it makes
    /// (logged by the harness's stubs, in order, with their arguments) - parked, straight in, or the 8-frame crossfade.
    #[test]
    fn clip_start_matches_the_original() {
        let mut n = 0;
        for (c, o) in walk_ref('c') {
            let mut t = o.split(' ');
            let o: Vec<u32> = t.by_ref().take(6).map(ref_word).collect();
            let log: String = t.map(|x| format!(" {x}")).collect();
            let mut s = ClipSlots { cur: c[0] as i32, prev: c[1] as i32, parked: c[2] as i32, oneoff: c[3], rate: f32::from_bits(c[4]), prev_rate: f32::from_bits(c[5]) };
            let (idx, rate) = (c[6] as i32, f32::from_bits(c[7]));
            let k = clip_start(&mut s, idx, rate, c[8] != 0);
            assert_eq!([s.prev as u32, s.cur as u32, s.parked as u32, s.oneoff, s.rate.to_bits(), s.prev_rate.to_bits()], [o[0], o[1], o[2], o[3], o[4], o[5]], "case {n}");
            let mut want = format!(" G{idx}");
            if k != ClipStart::Parked {
                want += &format!(" G{idx} A00000000/3f800000 L11 R{:08x} F{:x} F{:x}", rate.to_bits(), c[12], c[9]);
                if k == ClipStart::Blend {
                    want += &format!(" B00000000/{:08x} T{:x}/0 e d", CLIP_BLEND_RATE.to_bits(), c[10]);
                } else {
                    want += &format!(" T{:x}/0 d", c[10]);
                }
                want += &format!(" o s{:x} m40006 h n40006 d z", c[11]);
            }
            assert_eq!(log, want, "case {n}");
            n += 1;
        }
        assert_eq!(n, 3000);
    }

    /// The anim update F-29547's clip choice against the original's code on 3,000 cases (part e: every locomotion state,
    /// current clip, rate, one-off clip and parked index, the slides and clip speeds of part b): the one-off start, the
    /// clip start with its rate, the rate change and the parked retry it asks for, in order.
    #[test]
    fn anim_choice_matches_the_original() {
        let mut n = 0;
        for (e, o) in walk_ref('e') {
            let c = &e[5..];
            let f = |i: usize| f32::from_bits(c[i]);
            let v = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            let (a28, a36) = ([f(5), f(6), f(7)], [f(8), f(9), f(10)]);
            let clip = if c[2] == 2 { 0x5100 } else { 0 };
            let cmd = anim_choice(e[0], e[1] as i32, f32::from_bits(e[2]), e[3], clip, c[0], f(4), || slide_speed(v(11), v(14), f(17)), &a28, &a36);
            let mut want = match cmd {
                AnimCmd::Keep => String::new(),
                AnimCmd::OneOff(k) => format!(" O{k:x}"),
                AnimCmd::Start(k, r) => format!(" S{k}/{:08x}", r.to_bits()),
                AnimCmd::Rate(r) => format!(" Q{:08x}", r.to_bits()),
            };
            if e[4] as i32 != -1 {
                want += " K";
            }
            assert_eq!(o, want.trim_start(), "case {n}");
            n += 1;
        }
        assert_eq!(n, 3000);
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
        // F-84843 with the idle clip lengths seen live; the stop lengths recorded on the
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
        let path: Vec<Vec3> = (0..5).map(|k| Vec3::new(k as f32 * 3.0, 0.0, 0.0)).collect();
        // beside segment 0 facing along it: its middle 1.5 m ahead
        let m = next_rail(&path, Vec3::new(0.2, 0.0, 0.3), Vec3::X, 0.1).unwrap();
        assert!((m.t.x - 1.5).abs() < 1e-4 && m.valid());
        // at that middle: the next one (the rail to the nearest is no rail)
        let m = next_rail(&path, Vec3::new(1.5, 0.0, 0.3), Vec3::X, 0.1).unwrap();
        assert!((m.t.x - 4.5).abs() < 1e-4);
        // facing back: only the fallback of 3.77 rad for the nearest segment remains
        let m = next_rail(&path, Vec3::new(0.2, 0.0, 0.3), -Vec3::X, 0.1).unwrap();
        assert!((m.t.x - 1.5).abs() < 1e-4);
        // exactly on the line, heading straight at the middle: the only rails loop once round (2 pi): none
        assert!(next_rail(&path, Vec3::new(0.2, 0.0, 0.0), Vec3::X, 0.1).is_none());
        // at the end of the path: nothing
        assert!(next_rail(&path, Vec3::new(10.5, 0.0, 0.0), Vec3::X, 0.1).is_none());
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
