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
    /// the type's szAudioType (`mcAmbientType.xml`, the type's +176 hash): its sound object in game.dat, which F-39461
    /// plays the reactions of ("AMBIENT_PED_FA_001")
    audio: String,
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
    /// the shared Pivot clips (`Mped_turnL/R_90/180`) in the avoid list's order with their root's turn at the clip's end
    /// (from its start; `turn_pick` reads it)
    pivot: Vec<(Clip, Quat)>,
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

/// A walker's place, facing and clip time at a tick's start (presentation only: `Peds::prepare` draws the walker
/// between it and the tick's end by the render alpha, as the cars are - vehicle_pose.rs).
#[derive(Clone, Copy)]
struct Between {
    pos: Vec3,
    rot: Quat,
    time: f32,
    blend: f32,
    state: State,
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
    /// the walker as the tick before the last one left it (`Peds::begin_tick`): what `prepare` draws it between
    prev: Option<Between>,
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
    /// on the navigation mesh: the path request (ped +512) and the path held (behaviour +544), F-73065 / F-12434
    wp: WalkPath,
    /// the walker's own MWC (ped +384; the original's seed is not decoded: the port's from `seed`)
    mwc: crate::vehicles::parked::Mwc,
    /// the port's search of the request out: its nodes (empty: the search failed)
    query: Option<Vec<PathNode>>,
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
        // (the query MWC's u x 2^-23 x total; the first whose running rest is <= 0, the last one untested)
        let mut x = rand() * total;
        let mut pick = cands[cands.len() - 1].1;
        for (w, l) in &cands[..cands.len() - 1] {
            x -= w;
            if x <= 0.0 {
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

// ---- The walker's path from the path system and back: the request F-73065 and the take F-23649 in the walker's update
// (F-30466), the request object's step F-12434 at the manager frame's start (F-12278: submit F-40518, poll F-88789,
// fetch F-60370, deliver F-44435, release F-79836), its cancel F-39771 (slot 4) behind the give-up F-50795 and the
// release F-75945, the path's init F-57015 and the search's seed F-93347. Decode and evidence:
// docs/reference/people/walker_path_rule.md (tools/walker_path_reference.py).

/// A node of a found path as F-44435 reads it (its type word +0): 0 / 1 a point (+4), 2 a shared edge (+4 a, +16 b),
/// anything else no point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PathNode {
    Point(Vec3),
    Edge(Vec3, Vec3),
    Other,
}

/// The most points a path holds (F-44435 stops at 48).
const PATH_MAX: usize = 48;
/// F-73065: at least 16 updates between two requests of a walker.
const PATH_AGE: i32 = 16;

/// The request object (ped +512) and the path R (behaviour +544).
#[derive(Clone, Debug, Default)]
pub(crate) struct WalkPath {
    /// asked for (+8), the search running or failed (+9: the step's result was 0), a path fetched and held (+10)
    want: bool,
    wait: bool,
    got: bool,
    /// the start (+16 .. +76: the walker's matrix as words, row 2 negated when asked backwards), the type's filter (+80),
    /// the owner R set (+84), the length (+88: the type's +124 x the speed scale)
    mat: [u32; 16],
    filter: u32,
    owner: bool,
    len: f32,
    /// R: the points (+0, count +768, at most 48), delivered and not taken yet (+784), the edge fraction (+788), the
    /// speed scale (+792), the updates since the last request (+796)
    pts: Vec<Vec3>,
    fresh: bool,
    frac: f32,
    scale: f32,
    age: i32,
}

/// The path system as the request object meets it: the query's submit (F-40518), its poll (F-88789: -1 not ready,
/// 0 failed, 1 found), the found path (F-60370: `None` when the result has no path record) and the release (F-79836).
pub(crate) trait PathSys {
    fn submit(&mut self, filter: u32, len: f32, mat: &[u32; 16]);
    fn poll(&mut self) -> i32;
    fn fetch(&mut self) -> Option<Vec<PathNode>>;
    fn release(&mut self);
}

/// A word of the 4-lane vector unit with denormals flushed.
fn wflush(w: u32) -> u32 {
    vftz(f32::from_bits(w)).to_bits()
}

impl WalkPath {
    /// F-57015 (the Sidewalk behaviour's ctor): count, flags, age and fraction 0, the speed scale fmsubs(u, 0.3 x 2^-23,
    /// 0.15) + 1 with u the walker's MWC (ped +384) draw.
    pub(crate) fn new(mwc: &mut crate::vehicles::parked::Mwc) -> WalkPath {
        let u = (mwc.next() & 0x7F_FFFF) as f32;
        WalkPath { scale: u.mul_add(f32::from_bits(0x3319_999A), -f32::from_bits(0x3E19_999A)) + 1.0, ..Default::default() }
    }

    /// The path's points (R +0, +768).
    pub(crate) fn points(&self) -> &[Vec3] {
        &self.pts
    }

    /// F-73065: the request, from the walker's update. The age (+796) counts every call; asked only when no other walker
    /// asked in this manager frame (`stamp`, the game-wide 0x8294A540 against the frame counter), 16 updates after the
    /// last request, M+260 bits 0x4 (slide) and 0x2 (own clip) clear, and no path held (+10) nor a search failed (+9).
    /// The start is the walker's matrix `mat` (row 2 negated - 0 - x in the vector unit - when `rev`), the filter the
    /// type's +96, the length type +124 x the scale.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn request(&mut self, m_flags: u32, frame: u32, stamp: &mut u32, mat: &[u32; 16], rev: bool, filter: u32, max_speed: f32) {
        self.age = self.age.wrapping_add(1);
        if *stamp == frame || self.age < PATH_AGE || m_flags & 4 != 0 || m_flags & 2 != 0 || self.got || self.wait {
            return;
        }
        let mut m = *mat;
        if rev {
            for w in &mut m[8..12] {
                *w = wflush((0.0 - f32::from_bits(wflush(*w))).to_bits());
            }
        }
        self.filter = filter;
        self.mat = m;
        self.len = max_speed * self.scale;
        self.want = true;
        self.owner = true;
        *stamp = frame;
        self.age = 0;
    }

    /// F-23649: a path delivered (+784) is taken once: true when M+260 bits 0x4 / 0x2 are clear (the caller then asks the
    /// behaviour for the next rail F-95960, unloads the rails F-78813 and sets the new one F-48384); with a bit set it is
    /// dropped (+784 cleared, the rails kept).
    pub(crate) fn take(&mut self, m_flags: u32) -> bool {
        if !self.fresh {
            return false;
        }
        self.fresh = false;
        m_flags & 4 == 0 && m_flags & 2 == 0
    }

    /// F-12434 (the request object's slot 3, from F-12278 at the manager frame's start): an asked request is submitted
    /// (result 0), else the search is polled; +9 = (result == 0); a found one is fetched (+10 = a path record was there)
    /// and delivered into R (F-44435 with the walker's MWC), then the query released.
    pub(crate) fn step(&mut self, sys: &mut dyn PathSys, mwc: &mut crate::vehicles::parked::Mwc) {
        let res = if self.want {
            self.want = false;
            sys.submit(self.filter, self.len, &self.mat);
            0
        } else {
            sys.poll()
        };
        self.wait = res == 0;
        if res > 0 {
            let found = sys.fetch();
            self.got = found.is_some();
            if let Some(nodes) = found {
                if self.owner {
                    self.deliver(&nodes, mwc);
                }
            }
            sys.release();
        }
    }

    /// F-44435: the path into R: one fraction fmadds(u, 0.4 x 2^-23, 0.5) - 0.2 per path (u the walker's MWC draw), a
    /// point node as it is, an edge a + (b - a) x the fraction (vector unit), other nodes nothing; at most 48 points;
    /// +784 set.
    fn deliver(&mut self, nodes: &[PathNode], mwc: &mut crate::vehicles::parked::Mwc) {
        self.pts.clear();
        let u = (mwc.next() & 0x7F_FFFF) as f32;
        self.frac = u.mul_add(f32::from_bits(0x334C_CCCD), 0.5) - f32::from_bits(0x3E4C_CCCD);
        for n in nodes {
            match *n {
                PathNode::Point(a) => self.pts.push(a),
                PathNode::Edge(a, b) => self.pts.push(vmadd(vsub(b, a), self.frac, a)),
                PathNode::Other => {}
            }
            if self.pts.len() == PATH_MAX {
                break;
            }
        }
        self.fresh = true;
    }

    /// F-39771 (the request object's slot 4): flags cleared, the query released, the start the identity rows with a
    /// zero place, filter, owner and length 0.
    fn cancel(&mut self, sys: &mut dyn PathSys) {
        self.want = false;
        self.got = false;
        self.wait = false;
        sys.release();
        let one = 1.0f32.to_bits();
        self.mat = [one, 0, 0, 0, 0, one, 0, 0, 0, 0, one, 0, 0, 0, 0, 0];
        self.filter = 0;
        self.len = 0.0;
        self.owner = false;
    }

    /// F-50795: the path given up (a stop run out twice, the car dodge, no next rail): cancelled, no points, not fresh,
    /// fraction 0.
    pub(crate) fn give_up(&mut self, sys: &mut dyn PathSys) {
        self.cancel(sys);
        self.pts.clear();
        self.fresh = false;
        self.frac = 0.0;
    }

    /// F-75945 (the behaviour's deactivate): cancelled, no points, not fresh (the fraction kept).
    pub(crate) fn release(&mut self, sys: &mut dyn PathSys) {
        self.cancel(sys);
        self.pts.clear();
        self.fresh = false;
    }
}

/// F-93347 (in the submit F-40518) with F-39482's reseed: two draws of the global MWC (0x82860890); the query's MWC
/// (0x8290C340) = (first & 0x7FFFFFFF, second & 0x7FFFFFFF) as (multiplier word, carry). The replay flag 0x828608A4 that
/// would take another source is 0 in play.
pub(crate) fn path_seed(global: &mut crate::vehicles::parked::Mwc) -> crate::vehicles::parked::Mwc {
    let a = global.next() & 0x7FFF_FFFF;
    let b = global.next() & 0x7FFF_FFFF;
    crate::vehicles::parked::Mwc { lo: a, carry: b }
}

/// The search's nodes from the port's walk (`random_path`): the shared edges, the 32nd hop's centre as a point.
fn path_nodes(hops: &[(Vec3, Vec3, u32)]) -> Vec<PathNode> {
    hops.iter().map(|&(a, b, _)| if a == b { PathNode::Point(a) } else { PathNode::Edge(a, b) }).collect()
}

/// The walker's MWC (ped +384). How the original seeds it is not decoded (open): the port's from the walker's hash.
fn ped_mwc(seed: u32) -> crate::vehicles::parked::Mwc {
    crate::vehicles::parked::Mwc { lo: seed & 0x7FFF_FFFF, carry: seed.rotate_left(16) | 1 }
}

/// The port's path system for one walker. The original's search runs on the nav thread: submitted at the manager frame's
/// start after the request, it is polled from the next frame's start on - the earliest delivery, which the port takes
/// (the thread's own time is not measured: open). The search F-28561 itself is the port's walk `random_path` from the
/// request's place facing 0 - its row 2, seeded as F-93347 seeds the query's MWC (two draws of the global one), the
/// query's u x 2^-23 its draws; filter and length are carried, the walk does not read them.
struct PortPaths<'a> {
    mesh: &'a NavMesh,
    poly: &'a mut u32,
    global: &'a mut crate::vehicles::parked::Mwc,
    query: &'a mut Option<Vec<PathNode>>,
}

impl PathSys for PortPaths<'_> {
    fn submit(&mut self, _filter: u32, _len: f32, mat: &[u32; 16]) {
        let w = |k: usize| f32::from_bits(mat[k]);
        let pos = Vec3::new(w(12), w(13), w(14));
        let heading = -Vec3::new(w(8), w(9), w(10));
        let start = if self.mesh.polys[*self.poly as usize].contains(pos.x, pos.z) { *self.poly } else { self.mesh.find(pos, AI_SIDEWALK, 2.0).unwrap_or(*self.poly) };
        *self.poly = start;
        let mut q = path_seed(self.global);
        let hops = random_path(self.mesh, start, pos, heading, &mut || q.float());
        *self.query = Some(path_nodes(&hops));
    }
    fn poll(&mut self) -> i32 {
        match self.query {
            None => -1,
            Some(v) if v.is_empty() => 0,
            Some(_) => 1,
        }
    }
    fn fetch(&mut self) -> Option<Vec<PathNode>> {
        self.query.take()
    }
    fn release(&mut self) {
        *self.query = None;
    }
}

/// The path system as a cancel meets it: only the query's release.
struct QueryRelease<'a>(&'a mut Option<Vec<PathNode>>);

impl PathSys for QueryRelease<'_> {
    fn submit(&mut self, _: u32, _: f32, _: &[u32; 16]) {}
    fn poll(&mut self) -> i32 {
        -1
    }
    fn fetch(&mut self) -> Option<Vec<PathNode>> {
        None
    }
    fn release(&mut self) {
        *self.0 = None;
    }
}

/// The walker's path given up (F-50795).
fn give_up_path(p: &mut Ped) {
    p.wp.give_up(&mut QueryRelease(&mut p.query));
}

/// The walker's matrix as the request copies it (rows X, Y, Z of the drawn rotation, the place; w lanes 0).
fn ped_mat(p: &Ped) -> [u32; 16] {
    let mut m = [0u32; 16];
    for (r, v) in [p.rot * Vec3::X, p.rot * Vec3::Y, p.rot * Vec3::Z, p.pos].into_iter().enumerate() {
        m[4 * r] = v.x.to_bits();
        m[4 * r + 1] = v.y.to_bits();
        m[4 * r + 2] = v.z.to_bits();
    }
    m
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

/// F-10083's gate: the behaviour's flags M+260 with bit 0x1 (avoidance on) and neither 0x4 (sliding) nor 0x2 (a clip of
/// its own).
pub(crate) fn ahead_runs(m260: u32) -> bool {
    m260 & 0x4 == 0 && m260 & 0x2 == 0 && m260 & 0x1 != 0
}

/// How F-10083 reaches the blocker's obstacle from its type bits (the archetype's +16): a walker or traffic (& 0x1020)
/// through the entity at +128 (F-88657: the cab proxy +52 when +500 bit 0x20, else +496), passed on the left only; a
/// car (& 0xC) through +80's +128; furniture (& 0x12) through the dynamic cast F-75805's +140; none of these: nothing
/// in the way.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum AheadObst {
    Entity,
    Object,
    Cast,
}

pub(crate) fn ahead_kind(ty: u32) -> Option<AheadObst> {
    if ty & 0x1FE0 & 0xFFFF_F03F != 0 {
        Some(AheadObst::Entity)
    } else if ty & 0xC != 0 {
        Some(AheadObst::Object)
    } else if ty & 0x1E & 0xFFFF_FFF3 != 0 {
        Some(AheadObst::Cast)
    } else {
        None
    }
}

/// F-79428's request to the passing rail's builder F-92334 (its words +28 .. +44): clearance 0.25, the
/// sweep's extra word 0.3 + the type's +492 (single), the gate 0x4011361E, the sweep's mask
/// 0x383E, the flags 0x80 (the left rails) | 0x40 (the right ones) when both sides (not a walker or traffic);
/// 0x20 (the lane test) and 0x10 (ready) clear.
pub(crate) fn ahead_request(kind: AheadObst, type492: f32) -> (f32, f32, f32, u32, u8) {
    let both = kind != AheadObst::Entity;
    (f32::from_bits(0x3E80_0000), f32::from_bits(0x3E99_999A) + type492, f32::from_bits(0x4011_361E), 0x383E, 0x80 | if both { 0x40 } else { 0 })
}

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
    /// sphere 1 (F-59837 with k = 1): its centre (+48, T's place at the draw) and pending flag (+20); tested at the next
    /// manager frame's start, read by that frame's sidestep (F-26402)
    centre1: Vec3,
    pending1: bool,
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
fn walker_ground(p: &mut Ped, col: &WorldCollider, cur: Vec3, frames: u32, draw0: bool, draw1: bool, dt: f32) {
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
    // (sphere 1 drawn by the same F-59837: T's place, radius 0.667, mask 0x103A)
    if draw1 {
        g.centre1 = t_place;
        g.pending1 = true;
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

/// The wait timer (F-84843) from the idle clip's length `v`, drawn from the walker's own MWC (ped +384) as the original
/// does: `v` = 0 -> 2 + 2 u (u the 23-bit draw), 0 < v < 5 (and any v < 5 that is not 0) -> n v - 8 x 0.0333333351 with
/// n = (31-bit draw mod 3) + 1 and the product fused, else v - 8 x 0.0333333351.
fn stop_time(v: f32, mwc: &mut crate::vehicles::parked::Mwc) -> f32 {
    let blend = 8.0 * f32::from_bits(0x3D08_8889);
    if v == 0.0 {
        2.0 + 2.0 * mwc.float()
    } else if v < 5.0 {
        crate::physics::exact::fma((mwc.index(3) + 1) as f32, v, -blend)
    } else {
        -crate::physics::exact::fma(8.0, f32::from_bits(0x3D08_8889), -v)
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
/// The angry clips a walker of sex `female` may play (F-72336's gather): of the avoid list's entries (`entries`: AvoidType,
/// clip loaded; the list's order) those whose clip is there and whose type is 4 + bSex (AngryMale / AngryFemale; the sex is
/// the byte's low 8 bits). The entries' indices, in order.
pub(crate) fn angry_list(entries: &[(u32, bool)], female: bool) -> Vec<usize> {
    let want = 4 + female as u32;
    entries.iter().enumerate().filter(|(_, e)| e.1 && e.0 == want).map(|(i, _)| i).collect()
}

/// The walker's angry clip (F-72336): one of `angry_list` by the shared MWC (0x82860890), `(lo & 0x7FFFFFFF) % n`; none,
/// and no draw, when the list is empty. Returns the entry's index.
pub(crate) fn angry_pick(entries: &[(u32, bool)], female: bool, rng: &mut crate::vehicles::parked::Mwc) -> Option<usize> {
    let list = angry_list(entries, female);
    (!list.is_empty()).then(|| list[rng.index(list.len() as u32) as usize])
}

/// F-86566: `v` turned by the unit quaternion `q` as the original does it, 2 (q.v) q + (2 w^2 - 1) v + 2 w (q x v): the
/// scalar products fused as there, the three vector multiply-adds unfused (vmulfp / vmaddfp).
pub(crate) fn quat_rotate(q: Quat, v: Vec3) -> Vec3 {
    use crate::physics::exact::fma;
    let s = fma(v.z, q.z, fma(v.x, q.x, q.y * v.y)) * 2.0;
    let k = fma(q.w * q.w, 2.0, -1.0);
    let c = Vec3::new(fma(q.y, v.z, -(q.z * v.y)), fma(q.z, v.x, -(v.z * q.x)), fma(v.y, q.x, -(q.y * v.x)));
    let w2 = q.w * 2.0;
    let a = Vec3::new(q.x * s, q.y * s, q.z * s);
    let b = Vec3::new(v.x * k + a.x, v.y * k + a.y, v.z * k + a.z);
    Vec3::new(c.x * w2 + b.x, c.y * w2 + b.y, c.z * w2 + b.z)
}

/// The walker's turn clip (F-16288): over the avoid list's entries of AvoidType 3 (Pivot, either sex) whose clip is
/// there, the clip's turn - its root's rotation at the clip's end (F-21043; `entries`: AvoidType, that rotation) turning
/// -z, its xz angle from -z (F-32060) - minus `want`, wrapped once each way by 2 pi (fsel); the entry whose |difference|
/// is the smallest, below the start 1e8, strictly (the first of equals). Returns the entry and its
/// difference (what the walker still turns after the clip, M+252).
pub(crate) fn turn_pick(entries: &[(u32, Option<Quat>)], want: f32) -> Option<(usize, f32)> {
    const PI: f32 = 3.141_592_7;
    const TAU: f32 = 6.283_185_5;
    let mut best = 1.0e8_f32;
    let mut out = None;
    for (i, e) in entries.iter().enumerate() {
        let (3, Some(q)) = *e else { continue };
        let d = crate::aiv::ang_xz(quat_rotate(q, Vec3::NEG_Z), Vec3::NEG_Z) - want;
        let d = if -(d - PI) >= 0.0 { d } else { d - TAU };
        let d = if d + PI >= 0.0 { d } else { d + TAU };
        if d.abs() < best {
            best = d.abs();
            out = Some((i, d));
        }
    }
    out
}

/// The Pivot clip for the port's wanted turn `want` (its yaw, counter-clockwise about y; `yaw_between`) by `turn_pick`,
/// whose angles run the other way (F-32060's sign): asked with -want, its difference yaw_o - (-want) is want - the clip's
/// port yaw, the turn still to make after the clip in the port's sense. The clip's index in `pivot` and that rest.
fn pivot_pick(ty: &PedType, want: f32) -> Option<(usize, f32)> {
    let entries: Vec<(u32, Option<Quat>)> = ty.pivot.iter().map(|(_, q)| (3, Some(*q))).collect();
    turn_pick(&entries, -want)
}

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

/// F-17399's t alone (the arc-line-arc rail's nearest point F-16977 asks it for the line).
pub(crate) fn seg_t(a: Vec3, b: Vec3, w: Vec3) -> f32 {
    seg_nearest(a, b, w).0
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

/// The sidestep's constants (the image's words): |n.y| beyond this takes the walker's forward instead of the normal, and
/// the sidestep's slide time.
const SIDESTEP_STEEP: u32 = 0x3F7A_E148; // 0.98
const SIDESTEP_T: f32 = 0.25;
/// The type flags of a touching object that make the walker step aside (archetype +16).
pub(crate) const SIDESTEP_MASK: u32 = 0x103A;

/// One of the walker's 12 contact slots (I+2240, 96 bytes each) as the sidestep F-26402 reads it: the type flags of the
/// object touched (slot+0 -> +4 -> +16; 0 for an empty slot or no archetype), the contact normal (slot+32) and the depth
/// (slot+68).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SideSlot {
    pub flags: u32,
    pub n: Vec3,
    pub depth: f32,
}

/// F-29590: the sidestep's way out. `dir` made flat and normalised (F-41334: eps 1e-5, else +x), turned by 0, -45 and
/// +45 degrees about +y; each probed over l / cos by the ball of the walker's radius swept from U (r + 0.3) above the
/// walker along the turned way laid in the ground's plane ((U x way) x U), the touching object (`obj`) left out (F-13499 /
/// F-27454, as F-94813's probe but without the 10 m reach, the lane limit or the ground test). The first clear one
/// gives `dir` = way x l / cos.
pub(crate) fn sidestep_probe(w: &DodgeWalker, obj: bool, dir: &mut Vec3, l0: f32, world: &mut dyn DodgeWorld) -> bool {
    use crate::physics::exact::{fma, guest_cos, guest_sin};
    let d = crate::aiv::norm_or(Vec3::new(dir.x, 0.0, dir.z), f32::from_bits(DODGE_LANE_EPS), Vec3::X);
    let from = vmadd(w.u, w.r + f32::from_bits(DODGE_LIFT), w.p);
    let swz = |v: Vec3| (Vec3::new(v.y, v.z, v.x), Vec3::new(v.z, v.x, v.y));
    for th in DODGE_TURNS {
        let ang = th * f32::from_bits(DODGE_DEG);
        let (sn, cs) = (guest_sin(ang as f64) as f32, guest_cos(ang as f64) as f32);
        let way = Vec3::new(fma(d.z, sn, d.x * cs), d.y, fma(d.z, cs, -(d.x * sn)));
        let l = l0 / cs;
        let (u_yzx, u_zxy) = swz(w.u);
        let (w_yzx, w_zxy) = swz(way);
        let c1 = -vsub(vmul(w_yzx, u_zxy), vmul(w_zxy, u_yzx));
        let (c_yzx, c_zxy) = swz(c1);
        let n = -vsub(vmul(u_yzx, c_zxy), vmul(u_zxy, c_yzx));
        let n2 = vdot3(n, n);
        let y0 = vrsqrt(n2);
        let y0 = if y0 == f32::INFINITY { 0.0 } else { y0 };
        let n = vmul(n, Vec3::splat(vnr(n2, y0)));
        let to = vadd(vmul(n, Vec3::splat(l)), from);
        if world.sweep(from, to, w.r, obj).is_none() {
            *dir = vmul(way, Vec3::splat(l));
            return true;
        }
    }
    false
}

/// F-26402: the sidestep, every frame unless sliding or a clip of the walker's own plays (M+260 bits 0x4 / 0x2): over
/// the 12 contact slots in order, an object whose type flags have 0x103A is stepped away from - along the contact normal
/// by its depth + 0.05, or, with the normal steeper than |n.y| 0.98, along the walker's forward (F-50349: 0 - matrix row 2)
/// by its radius -, through F-29590; the first that finds a way starts the slide (F-83234: no car, T 0.25, clear 0.0, the
/// sidestep's flag: no clip pick, no speech). Returns whether it did.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sidestep(m: &mut Avoid, w: &DodgeWalker, fwd: Vec3, slots: &[SideSlot], items: &[AvoidItem], durs: &[f32], world: &mut dyn DodgeWorld, speech: &mut crate::vehicles::parked::Mwc, pick: &mut crate::vehicles::parked::Mwc) -> bool {
    if m.flags & 6 != 0 {
        return false;
    }
    for s in slots.iter().take(12) {
        if s.flags & SIDESTEP_MASK == 0 {
            continue;
        }
        let (mut dir, depth) = if s.n.y.abs() > f32::from_bits(SIDESTEP_STEEP) { (fwd, w.r) } else { (s.n, s.depth) };
        if sidestep_probe(w, true, &mut dir, depth + f32::from_bits(DODGE_INSIDE_R), world) {
            slide_start(m, w, items, durs, None, dir, s.n, SIDESTEP_T, 0.0, true, world, speech, pick);
            return true;
        }
    }
    false
}

// ---- The walker's sphere 1, the sidestep's contact fill (decode and evidence: docs/reference/people/car_dodge_rule.md
// section 12; tools/sphere_fill_reference.py). F-59837 draws it (centre T's place, radius 0.667, include mask 0x103A,
// material mask 512 << the material manager's flag shift, the walker's own instance left out); at the next frame's start
// F-78763 clears its 12 slots (F-96720) and queues the test (F-66549), which a worker job runs (F-52180 -> F-49353):
// every level object whose type flags meet 0x103A, in the level's cull order, is tested against its bound (F-75667;
// a walker's capsule: F-15615 case 1), each contact allocated a slot (F-89028: the 12 deepest kept) and the object's
// pending slots put into world space (F-77105 / F-57315). The sidestep F-26402 reads the slots in that frame.

/// The walker's capsule bound (F-37628 -> F-25220 / F-77704): radius 0.3, length 0x3F999999 (the image's 1.2 less one
/// step), centroid (0, 0.9, 0); its material (+208, F-60139) is 0.
const CAPSULE_R: u32 = 0x3E99_999A;
const CAPSULE_LEN: u32 = 0x3F99_9999;
const CAPSULE_MID_Y: u32 = 0x3F66_6666;
/// The walker's archetype type flags (0x20: what sphere 1's mask 0x103A selects of walkers).
pub(crate) const WALKER_TYPE: u32 = 0x20;
/// F-15615's guard on the squared distance before it divides (0x82008550) and its normal for a centre on the axis
/// (0x827D8470: +x).
const FILL_EPS: u32 = 0x2B8C_BCCC;
/// The slots of a sphere (F-38096's max 12).
pub(crate) const FILL_SLOTS: usize = 12;

/// One hit slot (a 96-byte record, F-15615 / F-89028): the object (`obj`, the caller's index), +16 the contact, +32 the
/// contact normal, +48 the bound's normal (the same for a capsule), +68 the depth, +76 the bound's part (capsule: 0 the
/// bottom cap, 1 the top, 2 the side), +72 the material, +80 depth < radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FillHit {
    pub obj: usize,
    pub pos: Vec3,
    pub n: Vec3,
    pub n2: Vec3,
    pub depth: f32,
    pub part: u16,
    pub material: u32,
    pub inside: bool,
}

/// A level object as the sphere test sees it: its instance matrix (rows and place, inst+16) and its capsule bound.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FillCapsule {
    pub obj: usize,
    pub rows: [Vec3; 3],
    pub pos: Vec3,
    pub r: f32,
    pub len: f32,
    pub mid: Vec3,
    pub material: u32,
}

impl FillCapsule {
    /// A walker's capsule at its instance matrix.
    pub(crate) fn walker(obj: usize, rows: [Vec3; 3], pos: Vec3) -> FillCapsule {
        let mid = Vec3::new(0.0, f32::from_bits(CAPSULE_MID_Y), 0.0);
        FillCapsule { obj, rows, pos, r: f32::from_bits(CAPSULE_R), len: f32::from_bits(CAPSULE_LEN), mid, material: 0 }
    }
}

/// The shape test's result list (F-96489 / F-16848 set it up): the slots, the committed count (+44), the pending count
/// (+52) and slots (+64 bits), and the highest / lowest score held (+80 / +84).
#[derive(Clone, Debug)]
pub(crate) struct FillList {
    pub slots: Vec<Option<FillHit>>,
    done: usize,
    pending: usize,
    pend: u32,
    hi: f32,
    lo: f32,
}

impl FillList {
    pub(crate) fn new(max: usize) -> FillList {
        FillList { slots: vec![None; max], done: 0, pending: 0, pend: 0, hi: -f32::MAX, lo: f32::MAX }
    }

    /// F-89028: a slot for a contact of `depth` (the sphere's radius `cap`) at `at`, or none. Its score is the depth, or
    /// cap - depth beyond the cap. A free slot while there is one; else the slot with the lowest score is overwritten
    /// when the new score beats the lowest held (+84), which then becomes the second lowest of the scan (the scan's
    /// second starts at the highest, +80; a list of one keeps the new score).
    fn alloc(&mut self, depth: f32, cap: f32, at: Vec3) -> Option<usize> {
        let max = self.slots.len();
        if depth <= 0.0 {
            return None;
        }
        // the point within the query's box (-FLT_MAX .. FLT_MAX: rejects NaN, and a point past it in all three)
        let ge = at.x >= -f32::MAX && at.y >= -f32::MAX && at.z >= -f32::MAX;
        if !ge || (at.x > f32::MAX && at.y > f32::MAX && at.z > f32::MAX) {
            return None;
        }
        let score = if depth > cap { cap - depth } else { depth };
        let n = self.done + self.pending;
        if n < max {
            self.pend |= 1 << n;
            self.hi = if score - self.hi >= 0.0 { score } else { self.hi };
            self.pending += 1;
            self.lo = if score - self.lo >= 0.0 { self.lo } else { score };
            return Some(n);
        }
        // (the original's compares: a NaN score or depth never takes a slot, as camprobe.rs::alloc - F-89028)
        if !(score > self.lo) {
            return None;
        }
        let (mut min, mut second, mut k) = (f32::MAX, self.hi, 0);
        for (j, s) in self.slots.iter().enumerate() {
            let d = s.map_or(0.0, |s| s.depth);
            let d = if d > cap { cap - d } else { d };
            if d < min {
                second = min;
                k = j;
                min = d;
            } else if d < second {
                second = d;
            }
        }
        if self.pend & (1 << k) == 0 {
            self.pend |= 1 << k;
            self.pending += 1;
            // (+72's bit is never set for this query: a committed slot counts in +44)
            self.done -= 1;
        }
        if max <= 1 {
            self.hi = score;
        }
        self.lo = if max > 1 { second } else { score };
        Some(k)
    }

    /// F-77105 (not counting only): the pending slots put into world space by the object's matrix (F-57315: the contact
    /// x rows + place as SSE dot products of four, the normals x rows of three), then committed.
    fn commit(&mut self, rows: &[Vec3; 3], pos: Vec3) {
        let cols = [Vec3::new(rows[0].x, rows[1].x, rows[2].x), Vec3::new(rows[0].y, rows[1].y, rows[2].y), Vec3::new(rows[0].z, rows[1].z, rows[2].z)];
        let place = [pos.x, pos.y, pos.z];
        // dpps 0xFF of (x, y, z, 1) with (c0, c1, c2, place): (1 place + z c2) + (y c1 + x c0), each step rounded, flushed
        let dot4 = |p: Vec3, c: Vec3, w: f32| {
            let q = |a: f32, b: f32| vftz(vftz(a) * vftz(b));
            vftz(vftz(q(1.0, w) + q(p.z, c.z)) + vftz(q(p.y, c.y) + q(p.x, c.x)))
        };
        for (i, s) in self.slots.iter_mut().enumerate() {
            if self.pend & (1 << i) == 0 {
                continue;
            }
            let Some(h) = s else { continue };
            h.pos = Vec3::new(dot4(h.pos, cols[0], place[0]), dot4(h.pos, cols[1], place[1]), dot4(h.pos, cols[2], place[2]));
            h.n = Vec3::new(vdot3(h.n, cols[0]), vdot3(h.n, cols[1]), vdot3(h.n, cols[2]));
            h.n2 = Vec3::new(vdot3(h.n2, cols[0]), vdot3(h.n2, cols[1]), vdot3(h.n2, cols[2]));
        }
        let max = self.slots.len().max(1);
        self.done = max.min(self.pending + self.done);
        self.pending = 0;
        self.pend = 0;
    }
}

/// F-15615 for a capsule (case 1), in the bound's frame: `c` the sphere's centre there (F-51913), `big_r` its radius.
/// d = c - centroid; the nearest axis point clamps d.y to +- half the length; within r + R of it a contact: normal =
/// (d.x, d.y - clamp, d.z) normalised (vrefp and a Newton step; +x when the distance is 1e-12 or less), depth = r + R -
/// distance, the contact c - n (R - depth), the part 1 above the axis' top, 2 along it, 0 below. The bound's material
/// against the query's material mask (+88) drops it; else it takes a slot (F-89028). Returns whether depth > 0.
fn capsule_hit(list: &mut FillList, c: Vec3, big_r: f32, cap: &FillCapsule, material_mask: u32) -> bool {
    use crate::physics::exact::fma;
    let d = vsub(c, cap.mid);
    let rc = cap.r;
    let h = cap.len * 0.5;
    let reach = rc + big_r;
    let lo = if d.y - -h >= 0.0 { d.y } else { -h };
    let yc = if lo - h >= 0.0 { h } else { lo };
    let dyr = d.y - yc;
    let d2 = fma(d.z, d.z, fma(dyr, dyr, d.x * d.x));
    let (depth, n, part) = if d2 > reach * reach {
        (-1.0f32, d, 0u16)
    } else {
        let (dist, n) = if d2 > f32::from_bits(FILL_EPS) {
            let dist = d2.sqrt();
            // vrefp, then y + y (1 - y dist) (vnmsubfp, vmaddfp)
            let y = vftz(1.0 / vftz(dist));
            let k = -vftz(vftz(y * dist) - 1.0);
            let inv = vftz(vftz(y * k) + y);
            (dist, vmul(Vec3::new(d.x, dyr, d.z), Vec3::splat(inv)))
        } else {
            (0.0, Vec3::X)
        };
        let part = if !(d.y < h) { 1 } else if d.y > -h { 2 } else { 0 };
        (reach - dist, n, part)
    };
    if cap.material & material_mask != 0 {
        return false;
    }
    let at = vnmsub(n, big_r - depth, c);
    if let Some(k) = list.alloc(depth, big_r, at) {
        list.slots[k] = Some(FillHit { obj: cap.obj, pos: at, n, n2: n, depth, part, material: cap.material, inside: depth < big_r });
    }
    depth > 0.0
}

/// The sphere test (F-49353's loop for one query) over `objs` in the order given (the original's: the level's cull order,
/// not decoded): each object's centre in its frame (F-51913: the dot products of centre - place with its rows), the
/// bound test (F-75667 -> F-15615), the pending slots committed in world space (F-77105). Returns the 12 slots.
pub(crate) fn sphere_fill(centre: Vec3, r: f32, objs: &[FillCapsule], material_mask: u32) -> Vec<Option<FillHit>> {
    let mut list = FillList::new(FILL_SLOTS);
    for o in objs {
        let c = vlocal(&o.rows, vsub(centre, o.pos));
        if capsule_hit(&mut list, c, r, o, material_mask) {
            list.commit(&o.rows, o.pos);
        }
    }
    list.slots
}

/// The sidestep's slots (F-26402's view) of a fill: the object's type flags (`flags_of`), the normal (+32), the depth.
pub(crate) fn side_slots(hits: &[Option<FillHit>], flags_of: impl Fn(usize) -> u32) -> Vec<SideSlot> {
    hits.iter().map(|h| h.map_or_else(SideSlot::default, |h| SideSlot { flags: flags_of(h.obj), n: h.n, depth: h.depth })).collect()
}

/// F-94626 (the walker's pose) in its one-off clip state (3): the turn taken off the clip's yaw correction M+252 this
/// frame (clamped to +- the turn rate x dt by two fsel) and the move along M+128 taken off the stretch M+256 (its positive
/// part x dt / max(T, dt), T = M+236). Returns ((the yaw left, the turn), (the stretch left, the move)); the walker's
/// matrix (the clip's) is then turned by the turn about +y (F-53646) and moved by M+128 x the move.
pub(crate) fn clip_correction(yaw: f32, stretch: f32, t: f32, dt: f32) -> ((f32, f32), (f32, f32)) {
    let lim = TURN_RATE * dt;
    let lo = if yaw - (-lim) >= 0.0 { yaw } else { -lim };
    let turn = if lo - lim >= 0.0 { lim } else { lo };
    let pos = if -stretch >= 0.0 { 0.0 } else { stretch };
    let den = if t - dt >= 0.0 { t } else { dt };
    let step = (pos * dt) / den;
    ((yaw - turn, turn), (stretch - step, step))
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
fn dodge_frame(p: &mut Ped, ty: &PedType, threats: &[Threat], side: &[SideSlot], col: Option<&WorldCollider>, nav: Option<&NavSys>, dt: f32, rand: &mut dyn FnMut() -> f32, rng: &mut DodgeRng, evades: &mut u32, speech: &mut Vec<Speech>, remark: &mut Remarks) -> bool {
    let clip = dodge_cars(p, ty, threats, side, col, nav, dt, rand, rng, evades, speech);
    // F-97113's last call, F-27362: the remark on the player's car (M+260 as the slide left it); a remark draws the
    // timer again (F-77233, the shared MWC)
    if let Some(kind) = remark_kind(p.dodge.flags, remark.timer, remark.car.as_ref(), p.pos) {
        speech.push(Speech { pos: p.pos, voice: ty.voice.clone(), female: ty.female, fast: false, kind, key: p.key, audio: ty.audio.clone() });
        remark.timer = remark_timer(REMARK_MIN, REMARK_MAX, &mut rng.pick);
    }
    clip
}

/// `dodge_frame` up to the remark: the car dodge, the sidestep and the slide.
#[allow(clippy::too_many_arguments)]
fn dodge_cars(p: &mut Ped, ty: &PedType, threats: &[Threat], side: &[SideSlot], col: Option<&WorldCollider>, nav: Option<&NavSys>, dt: f32, rand: &mut dyn FnMut() -> f32, rng: &mut DodgeRng, evades: &mut u32, speech: &mut Vec<Speech>) -> bool {
    let m = &mut p.dodge;
    // M+260 bit 0x2: a clip of the walker's own (the port's angry and Pivot clips)
    m.flags = (m.flags & !2) | if matches!(p.state, State::Angry(_) | State::Turn(_)) { 2 } else { 0 };
    let w = DodgeWalker { p: p.pos, r: PED_RADIUS, u: p.ground.up, rows: [p.rot * Vec3::X, p.rot * Vec3::Y, p.rot * Vec3::Z], female: ty.female };
    let cars: Vec<DodgeCar> = threats.iter().map(|t| t.car).collect();
    let durs: Vec<f32> = ty.evade.iter().map(|c| c.anim.duration).collect();
    let mut world = PedDodgeWorld { col, nav, evade: &ty.evade, stop: false, say: None, dropped: false };
    let re = m.clip.is_some() && m.flags & 2 == 0;
    car_dodge(m, &w, &cars, dt, &ty.avoid, &durs, &mut world, &mut rng.speech, &mut rng.pick);
    // the sidestep F-26402 (F-97113's next call): sphere 1's slots of this frame; the walker's forward F-50349 = 0 - row 2
    if !side.is_empty() {
        sidestep(m, &w, vneg(w.rows[2]), side, &ty.avoid, &durs, &mut world, &mut rng.speech, &mut rng.pick);
    }
    if world.stop {
        p.stop = stop_time(ty.idle.anim.duration, &mut p.mwc);
        p.stops += 1;
    }
    if let Some(k) = world.say {
        speech.push(Speech { pos: p.pos, voice: ty.voice.clone(), female: ty.female, fast: k == 0, kind: k, key: p.key, audio: ty.audio.clone() });
    }
    let m = p.dodge;
    let clip = m.clip.and_then(|e| ty.avoid.get(e)?.clip);
    if world.dropped {
        p.rail = None;
        give_up_path(p);
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
    // the slide (F-56160): M+240 += dt; the end (M+240 >= M+244) is not tested on the slide's first step (M+240 was 0.0
    // before it), so a slide shorter than a frame still lasts two
    let time = m.time + dt;
    p.dodge.time = time;
    let done = m.time != 0.0 && time >= m.len;
    if done {
        p.dodge.clip = None;
        p.dodge.flags &= !4;
    }
    if clip.is_none() || !matches!(p.state, State::Evade(_)) {
        // (no clip: the sidestep slide moves the walker)
        return false;
    }
    // (with a clip the walker is not put at M+112: F-94626 poses it by the clip's matrix and the corrections - the
    // caller's root motion)
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
fn sidewalk_step(p: &mut Ped, ty: &PedType, me: usize, peds: &[Vec3], props: &[Obst], col: Option<&WorldCollider>, nav: Option<&NavSys>, threats: &[Threat], side: &[SideSlot], due: u8, dt: f32, rand: &mut dyn FnMut() -> f32, rng: &mut DodgeRng, evades: &mut u32, speech: &mut Vec<Speech>, remark: &mut Remarks, frame: u32, stamp: &mut u32) -> f32 {
    // (the behaviour's facing: the rail's, kept apart from the drawn one, which is the mean of the last 8 frames')
    let fwd = if p.ground.face != Vec3::ZERO { p.ground.face } else { flat(p.rot * -Vec3::Z).normalize_or(Vec3::Z) };
    p.ground.t_face = fwd;
    let set_state = set_clip_state;
    p.hold = (p.hold - dt).max(0.0);
    // the car dodge, every frame (F-15352 from F-97113, no gate of its own): a dodge's clip slides the walker here
    let dodging = dodge_frame(p, ty, threats, side, col, nav, dt, rand, rng, evades, speech, remark);
    // then the path (F-30466 from the behaviour's update F-72378): the request F-73065 - backwards after an even number
    // of stops in a row, more than one - and the take F-23649 of a path delivered; a path taken drops the rails, the
    // next rail F-95960 turns round when the stops ask backwards. M+260: bit 0x4 sliding, 0x2 a clip of its own.
    let m_flags = p.dodge.flags | if p.slide.is_some() { 4 } else { 0 } | if matches!(p.state, State::Evade(_) | State::Angry(_) | State::Turn(_)) { 2 } else { 0 };
    let rev = p.stops > 1 && p.stops % 2 == 0;
    let mat = ped_mat(p);
    // (the filter type +96 and the length type +124 x scale are carried; the port's walk reads neither)
    p.wp.request(m_flags, frame, stamp, &mat, rev, AI_SIDEWALK as u32, 0.0);
    if p.wp.take(m_flags) {
        p.rail = None;
        p.ride = None;
        p.turn_round = rev && !p.wp.points().is_empty();
    }
    if dodging {
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
    let query = due & 4 != 0;
    let ahead = p.pos + fwd * (AHEAD_MAX * rand());
    // sliding aside
    if let Some((a, b, t0, len)) = p.slide {
        let t = t0 + dt;
        p.pos = a.lerp(b, (t / len).min(1.0));
        // (the target faces the push while sliding aside: F-94626 builds T's r2 from behaviour +400, the push F-83234
        // stores there)
        p.ground.t_face = flat(b - a).normalize_or(fwd);
        // (sliding, F-33058: walk or run by the slide's speed F-92375, which is the wanted speed)
        let s = slide_speed(a, b, len);
        let (st, want, lag) = locomotion(4, p.stop, true, p.scale, false, || s, &ty.a28, &ty.a36);
        p.speed += (want - p.speed) * (dt / lag).min(1.0);
        // (F-56160: no end on the slide's first step, when the time was 0.0)
        p.slide = if t0 != 0.0 && t >= len { None } else { Some((a, b, t, len)) };
        return loco_clip(p, ty, st, 4, s);
    }
    // a reversed path: turn round as it is taken (F-95960; a Pivot clip beyond
    // 45 degrees), which ends the wait
    if p.turn_round {
        p.turn_round = false;
        let want = yaw_between(fwd, -fwd);
        if let Some((k, rest)) = (want.abs() > std::f32::consts::FRAC_PI_4).then(|| pivot_pick(ty, want)).flatten() {
            p.turn_rest = rest;
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
    // (touching something: the sidestep F-26402 on sphere 1's slots, in `dodge_frame` after the car dodge; a slide it
    // starts is carried by the slide above)
    // something ahead (F-10083, run every frame the walker is not waiting, on a detour
    // rail too): a rail round it, or stop. The candidates (F-92334): the corner of the
    // box furthest left as seen from the walker, pushed out to the left by half the walker's
    // width and 0.25 (0.55); for furniture also the corner furthest right pushed right; a
    // walker in the way is only ever passed on the left (F-10083 clears args +44 bit 0x40
    // for type 0x20). Each is reached by a turn of at most 130 degrees (turning radius 0.1:
    // arcs too short to sample) and swept by the capsule all the way; the shortest wins.
    let hit = if query { sphere_hit(ahead, AWARE_RADIUS, me, peds, props, false) } else { None };
    if let Some((o, who)) = hit {
        // F-10083 -> F-79428 -> F-92334 (`traffic::pass_pick`): the request on F-79428's stack - the walker's start (its
        // rail at its phase, y = 0, made unit; with no rail its place and facing), the blocker's matrix and box
        // (slot 1, slots 4 / 6), the walker's own width 0.6 (2 x 0.3), clearance 0.25, the gate
        // 2.26893 (0x4011361E), no lane test, 0x80 the left rails and 0x40 the right ones only when the blocker is
        // not a walker or traffic (F-10083: type & 0x1020 -> r29 = 0); the radius F-36195 at the walker's speed with
        // its type's fMinTurningRadius. The sweep F-23371 (mask 0x383E, 0.3 + the type's +492 up) is the port's
        // capsule test along the rail's chords (its world, not the level's query).
        let (sp, sh) = match p.ride {
            Some((r, ph)) => {
                let mut t = r.tangent(ph);
                t.y = 0.0;
                (crate::tcp_rail::vadd3(r.point(ph), Vec3::ZERO), crate::tcp_rail::vnorm(t))
            }
            None => (crate::tcp_rail::vadd3(p.pos, Vec3::ZERO), fwd),
        };
        let row0 = o.1;
        // (the port's blockers: a walker, type 0x20, or furniture, 0x2)
        let ahead = ahead_request(if who == usize::MAX { AheadObst::Cast } else { AheadObst::Entity }, 0.0);
        let req = crate::vehicles::traffic::PassReq {
            p: sp,
            h0: sh,
            row0,
            row2: Vec3::new(-row0.z, 0.0, row0.x),
            place: o.0,
            w: o.2.x * 2.0,
            l: o.2.y * 2.0,
            w_own: 2.0 * PED_RADIUS,
            c: ahead.0,
            radius: crate::vehicles::traffic::radius_at(p.speed, ty.turn_radius),
            gate: ahead.2,
            flags: ahead.4,
        };
        let mut sweep = |q: &crate::tcp_rail::TcpRail| {
            let legs = [(q.p, q.a), (q.a, q.b), (q.b, q.t)];
            !legs.iter().all(|&(a, b)| a.distance(b) < 1e-4 || way_clear(a, b, AVOID_CLEAR, me, usize::MAX, None, peds, props, col, false, false))
        };
        let pass = crate::vehicles::traffic::pass_pick(&req, &mut sweep, &mut |_, _| Vec::new(), &mut |_| false, &mut |_, _| true);
        if std::env::var("MCLA_PED_DUMP").is_ok() {
            log::info!("ped avoid {} {} {} ahead {:.2}", p.key.1, if who == usize::MAX { "prop" } else { "ped" }, pass.is_some(), flat(o.0 - p.pos).dot(fwd));
        }
        match pass {
            Some(r) => {
                // (the rail ridden by phase to its end T, then the path's rails again)
                p.rail = Some((r.t, false));
                p.ride = Some((r, 0.0));
                p.stops = 0;
            }
            None => {
                if std::env::var("MCLA_PED_DUMP").is_ok() {
                    log::info!("ped stop {} by {} at {:.1} m, half {:.2} {:.2} centre {:.1} {:.1}", p.key.1, if who == usize::MAX { "prop" } else { "ped" }, box_dist(&o, p.pos), o.2.x, o.2.y, o.0.x, o.0.z);
                }
                p.stop = stop_time(ty.idle.anim.duration, &mut p.mwc);
                p.stops += 1;
                if expired && p.stops > 1 {
                    p.rail = None;
                    give_up_path(p);
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
            let pts: Vec<Vec3> = p.wp.points().to_vec();
            match next_rail(&pts, p.pos, heading, ty.turn_radius) {
                Some(r) => {
                    p.rail = Some((r.t, false));
                    p.ride = Some((r, 0.0));
                }
                None => {
                    // (F-95960 gives up a path it cannot rail; with no points held nothing is given up, so a request
                    // out is not cancelled - whether the follower asks F-95960 without a path is not decoded)
                    if !p.wp.points().is_empty() {
                        give_up_path(p);
                    }
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
    if p.rail.is_none() && p.wp.points().is_empty() {
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
/// sqrt(`r2`) from the first corner towards the far edge. The weights are the cross products' lengths
/// (sqrt of the three squares fused, summed in order, each times 1 / the total), the pick the first triangle
/// whose running sum is not below `r1`, t = (r1 - the sum before) / its weight, and the point
/// fma(v0, 1 - s, fma(v[k+2], s t, v[k+1] (1 - t) s)). When the rounded weights sum to under `r1` the original goes past
/// the last triangle and reads what the stack held; here that case takes the last (`random_point_k` says so).
pub(crate) fn random_point(q: &NavPoly, r1: f32, r2: f32) -> Vec3 {
    random_point_k(q, r1, r2).0
}

/// `random_point` and whether the original's pick ran off the end of the weights (its point is then not defined).
pub(crate) fn random_point_k(q: &NavPoly, r1: f32, r2: f32) -> (Vec3, bool) {
    use crate::physics::exact::fma;
    let v = q.points();
    if v.len() < 3 {
        return (q.center, false);
    }
    let n = v.len() - 2;
    let mut w = [0.0f32; 2];
    let mut total = 0.0f32;
    for k in 0..n {
        let c = (v[k + 1] - v[0]).cross(v[k + 2] - v[0]);
        w[k] = fma(c.z, c.z, fma(c.y, c.y, c.x * c.x)).sqrt();
        total = w[k] + total;
    }
    let inv = 1.0 / total;
    for x in w[..n].iter_mut() {
        *x = inv * *x;
    }
    let mut acc = 0.0f32;
    let mut k = 0;
    while k < n {
        acc = w[k] + acc;
        if !(acc < r1) {
            break;
        }
        k += 1;
    }
    let off = k == n;
    let k = k.min(n - 1);
    let t = ((r1 - acc) + w[k]) / w[k];
    let s = r2.sqrt();
    let (a, b) = (1.0 - s, (1.0 - t) * s);
    let c = s * t;
    let at = |f: fn(Vec3) -> f32| fma(f(v[0]), a, fma(f(v[k + 2]), c, f(v[k + 1]) * b));
    (Vec3::new(at(|p| p.x), at(|p| p.y), at(|p| p.z)), off)
}

#[derive(Clone, Copy)]
enum Plan {
    /// carry on along the next block (over the side street)
    Straight(usize, f32),
    /// over to the opposite pavement of the own street
    Cross(Vec3, usize, f32),
}

/// One object of the grid as the people spawner's room test reads it (F-70932): its kind (+6: 0 / 1
/// racers, 2 ambient cars, 3 props, 4 people), its place (slot 1, +48) and whether its owner's type
/// (+16 -> +448) is the spawner's.
#[derive(Clone, Copy, Debug)]
pub struct RoomObj {
    pub kind: u8,
    pub pos: Vec3,
    pub same_type: bool,
}

/// F-70932: no room at `p` when an object of the 3 x 3 grid cells round p's cell (F-88455, F-99153;
/// none when p is off the grid) of kind 0 .. 4 lies flat within `s1` (fMinSpawnPedSpacing), a person of
/// the same type within `s2` (fMinSpawnPedTypeSpacing): (o - p) over x and z squared, the z term fused,
/// below s x s.
pub fn room_blocked(grid: &crate::vehicles::traffic::CrossGrid, p: Vec3, objs: &[RoomObj], s1: f32, s2: f32) -> bool {
    let Some((cx, cz)) = grid.cell(p.x, p.z) else { return false };
    objs.iter().any(|o| {
        if o.kind > 4 {
            return false;
        }
        match grid.cell(o.pos.x, o.pos.z) {
            Some((ox, oz)) if (ox - cx).abs() <= 1 && (oz - cz).abs() <= 1 => {}
            _ => return false,
        }
        let s = if o.kind == 4 && o.same_type { s2 } else { s1 };
        let (dx, dz) = (o.pos.x - p.x, o.pos.z - p.z);
        crate::physics::exact::fma(dz, dz, dx * dx) < s * s
    })
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
    /// and vt+28 = sqrt(fma(hx, hx, hz hz)) as the racer's own extent writer has it; +416 = `prev`, the velocity the car
    /// had at the frame's start (`Threat::all`); the burnout charge `burn`.
    pub fn from_car(c: &crate::vehicles::CarInstance, prev: Vec3) -> Threat {
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
            prev,
            size: (h.x * 2.0, h.z * 2.0),
            rv: crate::physics::exact::fma(h.x, h.x, h.z * h.z).sqrt(),
            burn: c.state.veh.as_ref().map(|v| v.burn),
            inst: true,
        };
        Threat { pos: q, vel: c.state.vel, car }
    }

    /// The frame's threats: every placed, non-parked car (`from_car`). Their +416 (the previous velocity) is what the
    /// original's frame start F-13118 copies from +432 for every car on its lists before the frame's sub-steps run
    /// (with +352 = +288, the previous matrix): the velocity at the previous frame's walker update. `prev` keeps it per
    /// car slot; a car with none (new this frame) has prev = vel, as the original's reset / set pose (F-15222, F-86308)
    /// copy +432 to +416.
    pub fn all(cars: &[crate::vehicles::CarInstance], prev: &mut Vec<Option<Vec3>>) -> Vec<Threat> {
        prev.resize(cars.len(), None);
        let out = cars
            .iter()
            .zip(prev.iter())
            .filter(|(c, _)| c.parked.is_none() && c.placed)
            .map(|(c, p)| Threat::from_car(c, p.unwrap_or(c.state.vel)))
            .collect();
        for (c, p) in cars.iter().zip(prev.iter_mut()) {
            *p = Some(c.state.vel);
        }
        out
    }
}

pub struct Speech {
    pub pos: Vec3,
    pub voice: String,
    pub female: bool,
    pub fast: bool,
    /// F-39461's kind: 0 the QUICK shout, 1 the SLOW one, 2 the expensive car / bike remark, 3 the damaged car one
    pub kind: u32,
    /// the walker (`Peds::pos_of`: the sound follows it, the entity's +116 pointer) and its type's sound object
    pub key: (usize, u32),
    pub audio: String,
}

/// The player's car as the walkers' remark reads it (F-27362, F-47522); main.rs fills it each frame, None while there
/// is no player car (F-37636).
#[derive(Clone, Copy, Debug, Default)]
pub struct RemarkCar {
    /// the car's matrix row 3 (F-35658 +48) and velocity (F-98427)
    pub pos: Vec3,
    pub vel: Vec3,
    /// mcCarDamage +36 (the damage points) and its tune's +24 (MaxDamage); None without a damage object
    pub damage: Option<(f32, f32)>,
    /// the six paint areas' unpainted bytes (the set-up's look part +1608, stride 48; F-47522 counts a byte of 1 only)
    pub unpainted: [u8; 6],
    /// the vehicle table's +44 (Price) of the car's TypeName (F-45718)
    pub price: i32,
    /// the purchase record's +82 word (spent + 1) of the record at the garage's current slot (LocalOptions +1344, F-82698:
    /// the slot number, not the model, indexes the records here)
    pub spent_word: u32,
}

/// The remark timer's range, the manager's +0x64248 / +0x6424C as its constructor sets them.
pub const REMARK_MIN: f32 = 15.0;
pub const REMARK_MAX: f32 = 30.0;

/// F-27362's state: the player's car and the manager's remark timer (+0x64244: 0 at the manager's reset).
#[derive(Clone, Copy, Debug, Default)]
pub struct Remarks {
    pub car: Option<RemarkCar>,
    pub timer: f32,
}

/// F-47522: the car is one to remark on - no paint area unpainted, and the parts bought (the record's word - 1) or
/// the price over twice the price, at most 75000 (signed words; a price over 75000 always is).
pub fn remark_expensive(unpainted: [u8; 6], price: i32, spent_word: u32) -> bool {
    if unpainted.contains(&1) {
        return false;
    }
    let spent = (spent_word & 0xFFFF) as i32 - 1;
    let a = if spent > price { spent } else { price };
    let b = price.wrapping_shl(1);
    let b = if b > 75000 { 75000 } else { b };
    a > b
}

/// F-27362's decision for a walker at `ped` whose M+260 is `flags`: None (no remark) or the kind, 3 the damaged car
/// (points over a third of MaxDamage), 2 an expensive one (F-47522). The tests in the original's order and compares
/// (a NaN passes the "greater than" gates).
pub fn remark_kind(flags: u32, timer: f32, car: Option<&RemarkCar>, ped: Vec3) -> Option<u32> {
    use crate::physics::exact::fma;
    if flags & 4 != 0 {
        return None;
    }
    let c = car?;
    if timer != 0.0 {
        return None;
    }
    // the flat speed^2 at most 44.957 (0x4233D3FE)
    if fma(c.vel.x, c.vel.x, c.vel.z * c.vel.z) > f32::from_bits(0x4233_D3FE) {
        return None;
    }
    // the flat distance^2 at most 20.903 (0x41A739B9, 4.57^2)
    let dx = c.pos.x - ped.x;
    let dz = c.pos.z - ped.z;
    if fma(dz, dz, dx * dx) > f32::from_bits(0x41A7_39B9) {
        return None;
    }
    if (c.pos.y - ped.y).abs() > 3.0 {
        return None;
    }
    let (points, max) = c.damage?;
    if points > max * f32::from_bits(0x3EAA_AA9F) {
        Some(3)
    } else if remark_expensive(c.unpainted, c.price, c.spent_word) {
        Some(2)
    } else {
        None
    }
}

/// F-77233: the remark timer drawn again from the shared MWC, min + (max - min) x u23 x 2^-23 (the product first,
/// then one fused multiply-add).
pub fn remark_timer(min: f32, max: f32, mwc: &mut crate::vehicles::parked::Mwc) -> f32 {
    let u = (mwc.next() & 0x7F_FFFF) as f32;
    crate::physics::exact::fma((max - min) * u, f32::from_bits(0x3400_0000), min)
}

/// The manager frame's countdown of the remark timer (inline in F-96156, an fsel): t - dt, held at 0.
pub fn remark_countdown(t: f32, dt: f32) -> f32 {
    let t = t - dt;
    if -t >= 0.0 { 0.0 } else { t }
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
    /// MCLA_DRAW_STEPS: each drawn walker's key and drawn place this frame (main.rs logs them)
    pub draw_log: Option<Vec<((usize, u32), Vec3)>>,
    /// the ambient manager's front-end mode (F-95688, the start screen): the spawner has no
    /// visibility test (F-33044, A-29552); the focus and view are the camera's
    pub frontend: bool,
    /// the traffic region list's count (traffic.rs `Vehicles::traffic_regions`, M +1792; None: no traffic spawner):
    /// the unspawners' dispatcher F-75661 runs the people's unspawner F-39594 only while it is not 0
    pub traffic_regions: Option<usize>,
    /// each car slot's velocity at the previous frame's walker update (the threats' +416, `Threat::all`)
    pub threat_prev: Vec<Option<Vec3>>,
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
    /// the cars of the object grid for the spawn room test (F-70932): kind (0 the player, 1 racers, 2 the
    /// others, as `Threat`) and place, taken from the threats each update
    room_cars: Vec<(u8, Vec3)>,
    active: HashSet<(usize, u32)>,
    curb: HashMap<usize, Option<f32>>,
    tune: Tuning,
    timer: f32,
    filled: bool,
    rng: u32,
    pub speech: Vec<Speech>,
    /// the remark on the player's car (F-27362): main.rs sets `remarks.car` each frame
    pub remarks: Remarks,
    /// the reactions' sounds playing, each following its walker (main.rs)
    pub voices_playing: crate::sndobj::PedVoices,
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
    /// the manager frame of the last path request of any walker (0x8294A540, F-73065)
    path_stamp: u32,
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
            draw_log: std::env::var_os("MCLA_DRAW_STEPS").map(|_| Vec::new()),
            frontend: false,
            preroll: 0,
            traffic_regions: None,
            threat_prev: Vec::new(),
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
            room_cars: Vec::new(),
            tune: Tuning { spawn_inner: 95.0, spawn_outer: 140.0, unspawn: 150.0, density: 0.011, box_width: 100.0, ped_spacing: 1.0, type_spacing: 20.0, fov_mph: [25.0, 45.0], fov_deg: [89.9, 45.0] },
            timer: 0.0,
            filled: false,
            rng: 0x1234_5678,
            speech: Vec::new(),
            remarks: Remarks::default(),
            voices_playing: Default::default(),
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
            path_stamp: u32::MAX,
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
        for (a, fast, front, kind) in &self.avoid {
            if *kind == 3 {
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
                _ => {}
            }
        }
        // (the angry clips as F-72336 gathers them, `angry_list`: the avoid list's order, AvoidType 4 + bSex, the clip loaded)
        let table: Vec<(u32, bool)> = self.avoid_list.iter().map(|(n, ty, ..)| (*ty, self.avoid.iter().any(|(a, ..)| a.short() == *n))).collect();
        let angry: Vec<Clip> = angry_list(&table, female)
            .into_iter()
            .filter_map(|i| self.avoid.iter().find(|(a, ..)| a.short() == self.avoid_list[i].0))
            .map(|(a, fast, front, _)| Clip::bind(a, &model.ids, *fast, *front))
            .collect();
        // (the Pivot clips as F-16288 reads them: the avoid list's order, AvoidType 3, either sex, the clip loaded)
        let pivot: Vec<(Clip, Quat)> = self
            .avoid_list
            .iter()
            .filter(|(_, ty, ..)| *ty == 3)
            .filter_map(|(n, ..)| self.avoid.iter().find(|(a, ..)| a.short() == *n))
            .map(|(a, fast, front, _)| {
                let c = Clip::bind(a, &model.ids, *fast, *front);
                let q = c.mover(c.last_frame()).1 * c.mover(0.0).1.inverse();
                (c, q)
            })
            .collect();
        let mut lod_switch = [40.0, 80.0];
        if let Ok(t) = std::fs::read_to_string(self.tune_dir.join(name).join("mcAmbientRenderType.xml")) {
            let s = xml_value(&t, "fLodSwitchScale").unwrap_or(1.0);
            lod_switch = [xml_value(&t, "fHighToMedSwitch").unwrap_or(40.0) * s, xml_value(&t, "fMedToLowSwitch").unwrap_or(80.0) * s];
        }
        let ambient_type = std::fs::read_to_string(self.tune_dir.join(name).join("mcAmbientType.xml")).ok();
        let turn_radius = ambient_type.as_deref().and_then(|t| xml_value(t, "fMinTurningRadius")).unwrap_or(0.1);
        let audio = ambient_type.as_deref().and_then(|t| Some(t.split("<szAudioType>").nth(1)?.split('<').next()?.trim().to_string())).unwrap_or_default();
        // the avoid list as the pick reads it: an entry's clip only when it is one of this sex's evades (the other
        // sex's entries and the angry / pivot ones fail the pick's type test anyway)
        let avoid = self.avoid_list.iter().map(|(n, ty, dir, audio)| AvoidItem { ty: *ty, dir: *dir, clip: evade_names.iter().position(|e| e == n), audio: *audio }).collect();
        Some(PedType { female, voice: core, audio, model, walk, idle, run, a28, a36, evade, avoid, angry, pivot, lod_switch, turn_radius })
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

    /// Where walker `key` stands now (a sound of its reaction follows it).
    pub fn pos_of(&self, key: (usize, u32)) -> Option<Vec3> {
        self.list.iter().find(|p| p.key == key).map(|p| p.pos)
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
        // a person's type tune has no bIsTrailer: its +112 stays 0 from the type ctor F-63144
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
            // room (F-70932; the gate M+2116 is 1 from the ctor F-10264, its only writer): the cars and
            // the walkers of the grid's 3 x 3 cells round the point
            let (s1, s2) = (self.tune.ped_spacing, self.tune.type_spacing);
            let objs: Vec<RoomObj> = self.room_cars.iter().map(|&(kind, pos)| RoomObj { kind, pos, same_type: false }).chain(self.list.iter().map(|p| RoomObj { kind: 4, pos: p.pos, same_type: p.ty == ti })).collect();
            if room_blocked(&crate::vehicles::traffic::CrossGrid::UNBOUNDED, pos, &objs, s1, s2) {
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
            let id = self.next_id;
            self.next_id = self.next_id.wrapping_add(1);
            let seed = hash(id, pi);
            // the walking speed factor, drawn once from the walker's MWC (F-57015)
            let mut mwc = ped_mwc(seed);
            let wp = WalkPath::new(&mut mwc);
            let scale = wp.scale;
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
                cur: Vec::new(), prev: None,
                loiter: false,
                calm: 0.0,
                seed,
                plan: None,
                cross: None,
                wait: 0.0,
                poly: pi,
                travel: face,
                wp,
                mwc,
                query: None,
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
            let _ = writeln!(w, "{:.2}	P	{}	{:.2}	{:.2}	{:.2}	{}	{}	{:.2}	{}	{}	{}	{}", t, p.key.1, p.pos.x, p.pos.y, p.pos.z, (p.state == State::Walk) as u8, st, p.stop, p.stops, p.wp.points().len(), p.rail.map_or(0, |r| 1 + r.1 as u8), p.slide.is_some() as u8);
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
                    cur: Vec::new(), prev: None,
                    loiter,
                    calm: 0.0,
                    seed: h,
                    plan: None,
                    cross: None,
                    wait: 0.0,
                    poly: NO_POLY,
                    travel: face,
                    wp: WalkPath { scale: 1.0, ..Default::default() },
                    mwc: ped_mwc(h),
                    query: None,
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

    /// A game tick starts: each walker's pose now is what the next drawn frames start from (`prepare`). Every tick,
    /// so that a tick without `update` (a hold) draws the walkers still.
    pub fn begin_tick(&mut self) {
        for p in self.list.iter_mut() {
            p.prev = Some(Between { pos: p.pos, rot: p.rot, time: p.time, blend: p.blend, state: p.state });
        }
    }

    pub fn update(&mut self, gpu: &mut Gpu, nav: &NavSys, col: &WorldCollider, obstacles: &dyn Fn(Vec3, f32, &mut Vec<Obst>), center: Vec3, threats: &[Threat], signals: Option<(&crate::signals::Signals, f32)>, hour: f32, dt: f32) {
        self.hour = hour;
        if !self.enabled || self.dirs.is_empty() {
            return;
        }
        let mut frame0 = self.frame_no;
        // (the manager frame F-96156 counts the remark timer down before the walkers' updates)
        self.remarks.timer = remark_countdown(self.remarks.timer, dt);
        self.room_cars.clear();
        self.room_cars.extend(threats.iter().map(|t| (t.car.kind, t.pos)));
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
        // the walkers' capsules in the level at the frame's start (their instance matrices: the drawn rotation and place),
        // what sphere 1's test meets (F-49353 at the next frame's start; the level's cull order is not decoded: list order)
        let caps: Vec<FillCapsule> =
            self.list.iter().enumerate().map(|(k, p)| FillCapsule::walker(k, [p.rot * Vec3::X, p.rot * Vec3::Y, p.rot * Vec3::Z], p.pos)).collect();
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
                    dodge_frame(p, ty, threats, &[], Some(col), Some(nav), dt, &mut rand, &mut self.dodge_rng, &mut self.evades, &mut self.speech, &mut self.remarks);
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
            // sphere 1 (F-78763 at a new manager frame's start: the slots cleared, a pending draw tested - F-66549 / F-49353 -
            // the walker's own instance left out; its capsule material 0 never meets the material mask)
            let mut side: Vec<SideSlot> = Vec::new();
            // the path request object's step F-12434 (F-12278, before sphere 1's): submit what was asked, else poll the
            // search; a path found is delivered into the walker's path with its MWC
            if frames > 0 && on_mesh {
                if let Some(mesh) = self.mesh.as_ref() {
                    let mut sys = PortPaths { mesh, poly: &mut p.poly, global: &mut self.dodge_rng.pick, query: &mut p.query };
                    p.wp.step(&mut sys, &mut p.mwc);
                }
            }
            if frames > 0 && p.ground.pending1 {
                p.ground.pending1 = false;
                let c = p.ground.centre1;
                let near: Vec<FillCapsule> = caps.iter().filter(|o| o.obj != i && o.pos.distance_squared(c) < 9.0).copied().collect();
                side = side_slots(&sphere_fill(c, AWARE_RADIUS, &near, 0), |_| WALKER_TYPE);
            }
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
                rate = sidewalk_step(p, ty, i, &peds_pos, &boxes, Some(col), Some(nav), threats, &side, due, dt, &mut rand, &mut self.dodge_rng, &mut self.evades, &mut self.speech, &mut self.remarks, self.frame_no, &mut self.path_stamp);
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
                    let fast = ty.evade[best.1].fast || speed > 18.0;
                    self.speech.push(Speech { pos: p.pos, voice: ty.voice.clone(), female: ty.female, fast, kind: if fast { 0 } else { 1 }, key: p.key, audio: ty.audio.clone() });
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
                // a dodge's clip on the mesh (F-94626 in the walker's one-off clip state): the clip's matrix turned about +y
                // by what is left of the yaw correction M+252 (at most the turn rate x dt, F-53646 on its rows) and moved
                // along M+128 by the stretch M+256's share (fix39; the slide's own place M+112 is not the walker's then)
                if on_mesh && matches!(p.state, State::Evade(_)) {
                    let ((yaw, turn), (stretch, step)) = clip_correction(p.dodge.yaw, p.dodge.stretch, p.dodge.t, dt);
                    p.dodge.yaw = yaw;
                    p.dodge.stretch = stretch;
                    p.rot = (Quat::from_rotation_y(turn) * p.rot).normalize();
                    p.pos += p.dodge.dir * step;
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
                    // (F-72336: one of the type's angry clips by the shared MWC, `angry_pick`)
                    let angry = |rng: &mut crate::vehicles::parked::Mwc| State::Angry(rng.index(ty.angry.len() as u32) as usize);
                    p.state = if on_mesh {
                        // after a dodge: face the car (a Pivot clip beyond 45 degrees), then
                        // the angry clip, else wait (F-73407)
                        let f = flat(p.rot * -Vec3::Z).normalize_or(Vec3::Z);
                        match p.state {
                            State::Evade(_) => {
                                let want = p.dodged.map_or(0.0, |c| yaw_between(f, flat(c - p.pos).normalize_or(f)));
                                if let Some((k, rest)) = (want.abs() > std::f32::consts::FRAC_PI_4).then(|| pivot_pick(ty, want)).flatten() {
                                    p.turn_rest = rest;
                                    p.then_angry = true;
                                    State::Turn(k)
                                } else if !ty.angry.is_empty() {
                                    angry(&mut self.dodge_rng.pick)
                                } else {
                                    p.stop = stop_time(ty.idle.anim.duration, &mut p.mwc);
                                    p.stops += 1;
                                    State::Idle
                                }
                            }
                            State::Turn(_) if p.then_angry => {
                                p.then_angry = false;
                                if !ty.angry.is_empty() {
                                    angry(&mut self.dodge_rng.pick)
                                } else {
                                    p.stop = stop_time(ty.idle.anim.duration, &mut p.mwc);
                                    p.stops += 1;
                                    State::Idle
                                }
                            }
                            _ => State::Idle,
                        }
                    } else {
                        match p.state {
                            State::Evade(_) if !ty.angry.is_empty() && rand() < 0.45 => angry(&mut self.dodge_rng.pick),
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
                walker_ground(p, col, cur, frames, due & 1 != 0, due & 2 != 0, dt);
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
            cur: Vec::new(), prev: None,
            loiter: true,
            calm: 0.0,
            seed: hash(id, item),
            plan: None,
            cross: None,
            wait: 0.0,
            poly: NO_POLY,
            travel: Vec3::new(-heading.sin(), 0.0, -heading.cos()),
            wp: WalkPath { scale: 1.0, ..Default::default() },
            mwc: ped_mwc(hash(id, item)),
            query: None,
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

    /// Every walker's instance matrix (rows, place: F-86858 sets it at the end of its update), in the list's order.
    pub fn instance_matrices(&self) -> Vec<[Vec3; 4]> {
        self.list.iter().map(|p| [p.rot * Vec3::X, p.rot * Vec3::Y, p.rot * Vec3::Z, p.pos]).collect()
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
    /// `alpha`: the render alpha (vehicles.rs `render_alpha`) - each walker is drawn between its pose at the last tick's
    /// start and its end, as the cars are; the port's interpolation, not the original's (it draws each 30 Hz frame as
    /// its update left it). Its place and facing are blended, its clip time and cross-fade too while its clip runs on.
    pub fn prepare(&mut self, gpu: &Gpu, cam: Vec3, frustum: &Frustum, hidden: bool, alpha: f32) {
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
            // (drawn between the last tick's start and end: a jump of 2 m or more - a placement - is drawn at once;
            // the clip's time only while the same clip runs on, not over a restart)
            let (pos, rot) = match p.prev {
                Some(b) if alpha < 1.0 && b.pos.distance(p.pos) < 2.0 => {
                    if b.state == p.state && b.time <= p.time && (b.time != p.time || b.blend != p.blend) {
                        clip.local(&ty.model.bones, clip.frame(b.time + (p.time - b.time) * alpha, looped), &mut local);
                        let blend = if b.blend >= p.blend { b.blend + (p.blend - b.blend) * alpha } else { p.blend };
                        if blend > 0.0 && p.from.len() == local.len() {
                            blend_pose(&mut local, &p.from, (blend / p.blend_len).clamp(0.0, 1.0));
                        }
                    }
                    (b.pos.lerp(p.pos, alpha), b.rot.slerp(p.rot, alpha))
                }
                _ => (p.pos, p.rot),
            };
            if let Some(d) = self.draw_log.as_mut() {
                d.push((p.key, pos));
            }
            let lod = lod as usize;
            // (at night the four lights of its program, picked and eased as the original's
            // F-44222 does at the walker's place - only while it is drawn, as there)
            if let (Some((dynamic, frames)), Some(t)) = (&self.night_lights, &tables) {
                let f = crate::fillmap::body_now(pos.x, pos.z);
                let amb = self.amb_base + Vec3::new(f[0], f[1], f[2]);
                let fl = self.fig_lights.entry(p.key).or_insert_with(|| crate::figlight::FigureLight::new(t, pos, amb));
                fl.step(t, pos, *frames);
                fl.step_amb(amb, *frames);
                self.buf.next_lanes = Some(fl.lanes(pos, dynamic));
                // (MCLA_PED_AMB_OLD=1: the ambient as before, the scene's plus the fill map at once, unpacked)
                if std::env::var("MCLA_PED_AMB_OLD").is_err() {
                    self.buf.next_amb = Some(fl.amb_bytes());
                }
            }
            self.buf.emit(&ty.model, lod, &local, Mat4::from_rotation_translation(rot, pos));
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
    /// F-10083 with F-79428 against the original's code run offline (tools/ped_ahead_reference.py, 4,000 cases; the
    /// passing rail's builder F-92334 stubbed, verified on its own): the gate, the empty slot and the instance with no
    /// archetype, the type's class and the obstacle's chain, the request words F-92334 is handed, the stop on a refusal
    /// (M+96 = the obstacle's place) and the stop count's reset (M+260 & 7).
    #[test]
    fn walker_ahead_matches_the_original() {
        use super::{ahead_kind, ahead_request, ahead_runs, AheadObst};
        let h = |t: &str| u32::from_str_radix(&t.replace('_', ""), 16).unwrap();
        let mut n = [0usize; 3];
        for l in include_str!("testdata/ped_ahead_ref.txt").lines() {
            let (i, o) = l.split_once(" -> ").unwrap();
            let c: Vec<u32> = i.split(' ').map(h).collect();
            let (fl, slot, ty) = (c[0], c[1], c[2]);
            let mut want = String::new();
            let mut m260 = fl;
            let mut m96 = [0u32; 4];
            'run: {
                if !ahead_runs(fl) || slot == 2 {
                    break 'run;
                }
                let kind = if slot == 0 { None } else { ahead_kind(ty) };
                // (the chain to the obstacle: the harness's objects, 0x2800 the obstacle, 0x2500 the cab proxy)
                let obst = match kind {
                    None => None,
                    Some(AheadObst::Entity) if c[3] & 0x20 != 0 && c[4] != 0 => Some(0x2500),
                    Some(AheadObst::Entity) => (c[5] != 0).then_some(0x2800),
                    Some(AheadObst::Object) => (c[6] != 0 && c[5] != 0).then_some(0x2800),
                    Some(AheadObst::Cast) => {
                        want += " C 1 00000000 827e_5e1c 827e_a460";
                        (c[7] != 0 && c[5] != 0).then_some(0x2800)
                    }
                };
                let Some(ob) = obst else {
                    m260 &= 7;
                    n[0] += 1;
                    break 'run;
                };
                let (cl, lift, gate, mask, flags) = ahead_request(kind.unwrap(), f32::from_bits(c[8]));
                want += " P 820d_9d6c";
                want += &" 00000000".repeat(11);
                for w in [c[10], ob, c[11], cl.to_bits(), lift.to_bits(), gate.to_bits(), mask, (flags as u32) << 24] {
                    want += &format!(" {w:08x}");
                }
                if c[9] & 0xFF != 0 {
                    // (a detour taken: F-79428's tail resets the stops in a row, the port's `stops = 0`)
                    want += " F 1 S";
                    m260 &= 7;
                    n[1] += 1;
                } else {
                    want += " L T";
                    m96 = [c[12], c[13], c[14], 0x3F80_0000];
                    n[2] += 1;
                }
            }
            want += &format!(" M {m260:08x} {:08x} {:08x} {:08x} {:08x}", m96[0], m96[1], m96[2], m96[3]);
            assert_eq!(want.trim().replace('_', ""), o.replace('_', ""), "{l}");
        }
        assert_eq!(n, [n[0], 595, 405]);
    }

    use super::*;
    use crate::ambient::Tod;

    fn ref_word(t: &str) -> u32 {
        u32::from_str_radix(&t.replace('_', ""), 16).unwrap()
    }

    /// The path system as the offline run stubs it: the case's poll answer while a query is out, -1 otherwise; the fetch
    /// as the case's +188 / path words give it; each call logged as the harness logs it.
    struct RefPathSys<'a> {
        out: bool,
        poll: i32,
        found: Option<Vec<PathNode>>,
        log: &'a mut Vec<String>,
    }

    impl PathSys for RefPathSys<'_> {
        fn submit(&mut self, filter: u32, len: f32, mat: &[u32; 16]) {
            self.log.push("t1".into());
            self.log.push(format!("{filter:x}"));
            self.log.push(format!("{:x}", len.to_bits()));
            self.log.extend(mat.iter().map(|w| format!("{w:x}")));
            self.out = true;
        }
        fn poll(&mut self) -> i32 {
            self.log.push("t2".into());
            if self.out { self.poll } else { -1 }
        }
        fn fetch(&mut self) -> Option<Vec<PathNode>> {
            self.found.clone()
        }
        fn release(&mut self) {
            self.log.push("t3".into());
            self.out = false;
        }
    }

    /// The walker's path request, delivery and release against the original's code run offline frame by frame
    /// (tools/walker_path_reference.py -> testdata/walker_path_ref.txt): 600 walkers over 24 manager frames each - the
    /// request object's step F-12434 at the frame's start (submit, poll -1 / 0 / 1, fetch with and without a path record,
    /// delivery F-44435 of up to 52 nodes of every type into at most 48 points with the walker's MWC), the walker's
    /// F-30466 (the request F-73065 under every gate: the frame's stamp taken by another walker, the age, M's bits, a
    /// path held, a failed search; reversed; the take F-23649 with and without M's bits) and the give-up F-50795 /
    /// release F-75945 (the cancel F-39771) - and 1,000 seeds F-93347. Every logged call with its words and the whole
    /// state after each frame (R's count, flags, fraction, scale, age, points; the request's flags, start, filter, length;
    /// the stamp; the walker's MWC) must be equal.
    /// The walker's remark on the player's car against the original's code run offline (tools/walker_remark_reference.py
    /// -> testdata/walker_remark_ref.txt): 20,000 walker / car states, half past every gate, the others near each
    /// (M+260 bit 0x4, the player, the timer 0 / -0 / tiny / running, the flat speed^2 by 44.957, the flat distance^2 by
    /// 20.903, |dy| by 3, the car's damage object), the damage by a third of MaxDamage, the six unpainted bytes 0 / 1 / 2 /
    /// 0xFF, prices and record words at F-47522's edges (signed words, 2 x price by 75000): the kind F-27362 asks of
    /// F-39461 (with the walker as its object), the timer after (F-77233 from the shared MWC) and the MWC after equal.
    #[test]
    fn walker_remark_matches_the_original() {
        let mut n = [0u32; 4];
        for l in include_str!("testdata/walker_remark_ref.txt").lines() {
            let (ci, o) = l.split_once(" -> ").unwrap();
            let c: Vec<u32> = ci.split(' ').map(ref_word).collect();
            let want: Vec<u32> = o.split(' ').map(ref_word).collect();
            let f = |i: usize| f32::from_bits(c[i]);
            let v = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            let car = RemarkCar {
                pos: v(8),
                vel: v(5),
                damage: (c[3] != 0 && c[4] != 0).then(|| (f(14), f(15))),
                unpainted: [0, 1, 2, 3, 4, 5].map(|k| c[16 + k] as u8),
                price: c[22] as i32,
                spent_word: c[24] & 0xFFFF,
            };
            let mut timer = f(1);
            let mut mwc = crate::vehicles::parked::Mwc { lo: c[25], carry: c[26] };
            let kind = remark_kind(c[0], timer, (c[2] != 0).then_some(&car), v(11));
            if kind.is_some() {
                timer = remark_timer(REMARK_MIN, REMARK_MAX, &mut mwc);
            }
            let k = kind.unwrap_or(0);
            n[k as usize] += 1;
            assert_eq!(want, vec![k, kind.is_some() as u32, timer.to_bits(), mwc.lo, mwc.carry], "{l}");
        }
        assert_eq!(n.iter().sum::<u32>(), 20_000);
        assert!(n[2] > 1000 && n[3] > 1000, "{n:?}");
    }

    #[test]
    fn walker_path_matches_the_original() {
        let (mut frames, mut taken, mut deliv, mut seeds) = (0, 0, 0, 0);
        for l in include_str!("testdata/walker_path_ref.txt").lines() {
            let (ci, o) = l.split_once(" ->").unwrap();
            let c: Vec<u32> = ci.split(' ').skip(1).map(ref_word).collect();
            if l.starts_with('1') {
                let want: Vec<u32> = o.split_whitespace().map(ref_word).collect();
                let mut g = crate::vehicles::parked::Mwc { lo: c[0], carry: c[1] };
                let q = path_seed(&mut g);
                assert_eq!(want, vec![f32::MAX.to_bits(), c[2], q.lo, q.carry, g.lo, g.carry], "{l}");
                seeds += 1;
                continue;
            }
            let f = |i: usize| f32::from_bits(c[i]);
            let v = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            let nn = c[22] as usize;
            let nodes: Vec<PathNode> = (0..nn)
                .map(|k| {
                    let o = 23 + 7 * k;
                    match c[o] {
                        0 | 1 => PathNode::Point(v(o + 1)),
                        2 => PathNode::Edge(v(o + 1), v(o + 4)),
                        _ => PathNode::Other,
                    }
                })
                .collect();
            let fr0 = 23 + 7 * nn;
            let mut mwc = crate::vehicles::parked::Mwc { lo: c[2], carry: c[3] };
            let mut wp = WalkPath { scale: f(0), age: c[1] as i32, ..Default::default() };
            let mat: [u32; 16] = c[6..22].try_into().unwrap();
            let mut stamp = 0u32;
            let mut out = false;
            for (k, want) in o.split(" |").enumerate() {
                let fr = &c[fr0 + 9 * k..fr0 + 9 * k + 9];
                let mut log = Vec::new();
                let found = (fr[1] != 0).then(|| if fr[8] != 0 { nodes.clone() } else { Vec::new() });
                let mut sys = RefPathSys { out, poll: fr[0] as i32, found, log: &mut log };
                wp.step(&mut sys, &mut mwc);
                let frame = 100 + k as u32;
                if fr[4] != 0 {
                    stamp = frame;
                }
                wp.request(fr[2], frame, &mut stamp, &mat, fr[3] != 0, c[5], f(4));
                if wp.take(fr[2]) {
                    sys.log.push("t6".into());
                    sys.log.push("t4".into());
                    sys.log.push("t5".into());
                    sys.log.push(format!("{:x}", fr[6]));
                    sys.log.push(format!("{:x}", fr[7]));
                    taken += 1;
                }
                if fr[5] == 1 {
                    wp.give_up(&mut sys);
                }
                if fr[5] == 2 {
                    wp.release(&mut sys);
                }
                out = sys.out;
                deliv += (!wp.pts.is_empty()) as usize;
                let mut got = log;
                let w = |x: u32| format!("{x:x}");
                got.extend([wp.pts.len() as u32, wp.fresh as u32, wp.frac.to_bits(), wp.scale.to_bits(), wp.age as u32].map(w));
                got.extend([wp.want as u32, wp.wait as u32, wp.got as u32, wp.filter, wp.owner as u32, wp.len.to_bits(), 0].map(w));
                got.extend(wp.mat.iter().map(|&x| w(x)));
                got.extend([stamp, mwc.lo, mwc.carry].map(w));
                for p in &wp.pts {
                    got.extend([p.x.to_bits(), p.y.to_bits(), p.z.to_bits()].map(w));
                }
                let want: Vec<String> = want.split_whitespace().map(|t| if t.starts_with('t') { t.to_string() } else { w(ref_word(t)) }).collect();
                assert_eq!(got, want, "frame {k} of {ci}");
                frames += 1;
            }
        }
        assert_eq!((frames, seeds), (600 * 24, 1000));
        assert!(taken > 200 && deliv > 200, "{taken} {deliv}");
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

    /// The angry clip pick (F-72336) against the original's code run offline on 4,000 cases: the entry and the MWC after
    /// must be equal.
    /// The turn clip choice (F-16288 with F-86566 and F-32060) against the original's code run offline on 4,000 cases
    /// (tools/turn_pick_reference.py -> testdata/turn_pick_ref.txt): lists of up to 8 entries, mostly Pivot, clips present
    /// or not, turns about y (quarter and half turns, near them, near the wanted turn) with a little tilt. The entry and
    /// the rest angle must be equal bit for bit.
    #[test]
    fn turn_pick_matches_the_original() {
        let h = |t: &str| u32::from_str_radix(t, 16).unwrap();
        let f = |t: &str| f32::from_bits(h(t));
        let (mut n, mut picked) = (0, 0);
        for l in include_str!("testdata/turn_pick_ref.txt").lines() {
            let (c, o) = l.split_once(" -> ").unwrap();
            let c: Vec<&str> = c.split(' ').collect();
            let o: Vec<&str> = o.split(' ').collect();
            let k = |i: usize, j: usize| 2 + 6 * i + j;
            let entries: Vec<(u32, Option<Quat>)> = (0..h(c[0]) as usize)
                .map(|i| (h(c[k(i, 0)]), (h(c[k(i, 1)]) != 0).then(|| Quat::from_xyzw(f(c[k(i, 2)]), f(c[k(i, 3)]), f(c[k(i, 4)]), f(c[k(i, 5)])))))
                .collect();
            let got = turn_pick(&entries, f(c[1]));
            let (e, r) = got.map_or((-1, u32::MAX), |(e, r)| (e as i32, r.to_bits()));
            assert_eq!((e, r), (o[0].parse::<i32>().unwrap(), h(o[1])), "{l}");
            n += 1;
            picked += (e >= 0) as usize;
        }
        assert_eq!((n, picked), (4000, 2897));
    }

    #[test]
    fn angry_pick_matches_the_original() {
        // (tools/angry_pick_reference.py -> testdata/angry_pick_ref.txt: lists of up to 10 entries of every AvoidType, clips
        // present or not, the sex byte (its low 8 bits count), the shared MWC)
        let h = |t: &str| u32::from_str_radix(&t.replace('_', ""), 16).unwrap();
        let (mut n, mut picked) = (0, 0);
        for l in include_str!("testdata/angry_pick_ref.txt").lines() {
            let (c, o) = l.split_once(" -> ").unwrap();
            let c: Vec<&str> = c.split(' ').collect();
            let o: Vec<&str> = o.split(' ').collect();
            let entries: Vec<(u32, bool)> = (0..h(c[0]) as usize).map(|i| (h(c[4 + 2 * i]), h(c[5 + 2 * i]) != 0)).collect();
            let mut rng = crate::vehicles::parked::Mwc { lo: h(c[2]), carry: h(c[3]) };
            let e = angry_pick(&entries, h(c[1]) & 0xFF != 0, &mut rng).map_or(-1, |e| e as i32);
            assert_eq!((e, rng.lo, rng.carry), (o[0].parse::<i32>().unwrap(), h(o[1]), h(o[2])), "{l}");
            n += 1;
            picked += (e >= 0) as usize;
        }
        assert_eq!((n, picked), (4000, 2276));
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

    /// The world of a sidestep reference case (tools/sidestep_reference.py): the swept balls' answers in order, every
    /// call logged as the harness does; the object a sweep leaves out is the slot being probed (the eligible slots in
    /// order, three turns each until a clear one).
    struct SideCase<'a> {
        c: &'a [u32],
        log: Vec<String>,
        sw: usize,
        queue: Vec<u32>,
        cur: usize,
        k: usize,
        mc: [u32; 8],
    }

    const SC_SW: usize = 44;
    const SC_ENT: usize = 85;
    const SC_SLOT: usize = 151;
    const SC_X: usize = 247;

    impl SideCase<'_> {
        fn w(&mut self, t: u32, ws: &[u32]) {
            self.log.push(format!("t{t}"));
            self.log.extend(ws.iter().map(|w| format!("{w:x}")));
        }
        fn v(&self, i: usize) -> Vec3 {
            Vec3::new(f32::from_bits(self.c[i]), f32::from_bits(self.c[i + 1]), f32::from_bits(self.c[i + 2]))
        }
    }

    impl DodgeWorld for SideCase<'_> {
        fn inside(&mut self, _c: Vec3, _r: f32) -> bool {
            self.log.push("unexpected-inside".into());
            false
        }
        fn sweep(&mut self, a: Vec3, b: Vec3, r: f32, car: bool) -> Option<(Vec3, u32)> {
            let obj = if car { self.queue.get(self.cur).copied().unwrap_or(u32::MAX) } else { 0 };
            self.w(3, &[a.x.to_bits(), a.y.to_bits(), a.z.to_bits(), b.x.to_bits(), b.y.to_bits(), b.z.to_bits(), r.to_bits(), obj]);
            let hit = self.c[SC_SW + self.sw] != 0;
            self.sw += 1;
            self.k += 1;
            if hit && self.k == 3 {
                self.cur += 1;
                self.k = 0;
            }
            hit.then_some((Vec3::ZERO, 0))
        }
        fn ground(&mut self, _a: Vec3, _b: Vec3) -> bool {
            self.log.push("unexpected-ground".into());
            false
        }
        fn lane(&mut self, _x: Vec3, _r: f32) -> Option<(Vec3, f32)> {
            self.log.push("unexpected-lane".into());
            None
        }
        fn mover(&mut self, clip: usize, t: f32) -> Vec3 {
            self.w(6, &[clip as u32, t.to_bits()]);
            let k = if self.mc[clip] > 0 { 1 } else { 0 };
            self.mc[clip] += 1;
            self.v(SC_ENT + 11 * clip + 5 + 3 * k)
        }
        fn stop(&mut self) {
            self.log.push("unexpected-stop".into());
        }
        fn speech(&mut self, kind: u32) {
            self.w(7, &[kind]);
        }
        fn drop_rails(&mut self) {
            self.w(8, &[]);
            self.w(9, &[]);
        }
    }

    /// The walkers' sidestep against the original's code run offline (tools/sidestep_reference.py ->
    /// testdata/sidestep_ref.txt): F-26402 whole on the 12 contact slots (empty, no archetype, type flags with and without
    /// 0x103A, normals about |n.y| 0.98 a float step either side, zero normals, depths below zero), with the probe F-29590
    /// and the slide start F-83234 inside it, the avoidance part's flags 0x2 / 0x4 set or not and an evade clip set or not;
    /// F-29590 called on its own (zero and steep directions). Every sweep (its words, in order), the avoidance part's words
    /// and the two MWCs must be equal.
    /// The walkers' sphere-1 fill against the original's code run offline (tools/sphere_fill_reference.py: F-38096,
    /// F-51913, F-15615 on capsule bounds, F-77105 / F-57315 / F-89028 inside, F-60139): every word of the 12 slots and the
    /// list's counts and scores equal on 3,000 cases (274 with the list full).
    #[test]
    fn sphere_fill_matches_the_original() {
        let h = |t: &str| u32::from_str_radix(&t.replace('_', ""), 16).unwrap();
        let (mut cases, mut full, mut slots) = (0, 0, 0);
        for l in include_str!("testdata/sphere_fill_ref.txt").lines() {
            let (ci, o) = l.split_once(" -> ").unwrap();
            let c: Vec<u32> = ci.split(' ').map(h).collect();
            let want: Vec<u32> = o.split(' ').map(h).collect();
            let f = |i: usize| f32::from_bits(c[i]);
            let v = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            let objs: Vec<FillCapsule> = (0..c[5] as usize)
                .map(|k| {
                    let o = 6 + 18 * k;
                    FillCapsule { obj: k, rows: [v(o), v(o + 3), v(o + 6)], pos: v(o + 9), r: f(o + 12), len: f(o + 13), mid: v(o + 14), material: c[o + 17] }
                })
                .collect();
            let mut list = FillList::new(FILL_SLOTS);
            for o in &objs {
                let lc = vlocal(&o.rows, vsub(v(0), o.pos));
                if capsule_hit(&mut list, lc, f(3), o, c[4]) {
                    list.commit(&o.rows, o.pos);
                }
            }
            assert_eq!(sphere_fill(v(0), f(3), &objs, c[4]), list.slots);
            let mut got = Vec::new();
            let vw = |o: &mut Vec<u32>, v: Vec3| o.extend([v.x.to_bits(), v.y.to_bits(), v.z.to_bits()]);
            for s in &list.slots {
                match s {
                    Some(s) => {
                        got.push(s.obj as u32 + 1);
                        vw(&mut got, s.pos);
                        vw(&mut got, s.n);
                        vw(&mut got, s.n2);
                        got.extend([0, s.depth.to_bits(), s.material, s.part as u32, 2, s.inside as u32, 0]);
                        slots += 1;
                    }
                    None => got.extend([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFFFF, 0, 0, 0]),
                }
            }
            got.extend([list.done as u32, list.pending as u32, list.hi.to_bits(), list.lo.to_bits()]);
            assert_eq!(got, want, "{l}");
            full += list.slots.iter().all(|s| s.is_some()) as usize;
            cases += 1;
        }
        assert_eq!((cases, full, slots), (3000, 274, 12276));
    }

    #[test]
    fn sidestep_matches_the_original() {
        let h = |t: &str| u32::from_str_radix(&t.replace('_', ""), 16).unwrap();
        let (mut n, mut slid, mut found) = ([0usize; 2], 0, 0);
        for l in include_str!("testdata/sidestep_ref.txt").lines() {
            let (ci, o) = l.split_once(" -> ").unwrap();
            let c: Vec<u32> = ci.split(' ').map(h).collect();
            let want: Vec<String> = o.split(' ').map(|t| if t.starts_with('t') { t.to_string() } else { format!("{:x}", h(t)) }).collect();
            let f = |i: usize| f32::from_bits(c[i]);
            let v = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            let w = DodgeWalker { p: v(1), r: f(4), u: v(5), rows: [v(8), v(11), v(14)], female: c[17] != 0 };
            let slots: Vec<SideSlot> = (0..12)
                .map(|k| {
                    let s = SC_SLOT + 8 * k;
                    SideSlot { flags: if c[s] != 0 && c[s + 1] != 0 { c[s + 2] } else { 0 }, n: v(s + 3), depth: f(s + 7) }
                })
                .collect();
            let queue = (0..12u32).filter(|&k| slots[k as usize].flags & SIDESTEP_MASK != 0).map(|k| k + 1).collect();
            let mut world = SideCase { c: &c, log: Vec::new(), sw: 0, queue, cur: 0, k: 0, mc: [0; 8] };
            let mut out: Vec<u32> = Vec::new();
            let vw = |o: &mut Vec<u32>, v: Vec3| o.extend([v.x.to_bits(), v.y.to_bits(), v.z.to_bits()]);
            if c[0] == 4 {
                let x = SC_X;
                world.queue = vec![1];
                let mut dir = v(x);
                let r = sidestep_probe(&w, c[x + 4] != 0, &mut dir, f(x + 3), &mut world);
                found += r as usize;
                out.push(r as u32);
                vw(&mut out, dir);
            } else {
                let nent = c[84] as usize;
                let items: Vec<AvoidItem> = (0..nent)
                    .map(|i| {
                        let e = SC_ENT + 11 * i;
                        AvoidItem { ty: c[e], dir: c[e + 1], audio: c[e + 2], clip: (c[e + 3] != 0).then_some(i) }
                    })
                    .collect();
                let durs: Vec<f32> = (0..6).map(|i| f(SC_ENT + 11 * i + 4)).collect();
                let mut m = Avoid {
                    car: Some(usize::MAX),
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
                let r = sidestep(&mut m, &w, v(41), &slots, &items, &durs, &mut world, &mut speech, &mut pick);
                slid += r as usize;
                let car = m.car.map_or(0, |_| 0xABCD);
                out.extend([(car == 0xABCD) as u32, car]);
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
            n[c[0] as usize - 3] += 1;
        }
        assert_eq!((n, slid, found), ([1600, 400], 699, 226));
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

    /// F-76960 run offline on the original's code (tools/random_point_reference.py -> testdata/random_point_ref.txt): 6,000
    /// triangles and quads (1 - 60 m, tilted or flat) with a global MWC state each; the point (the function's two draws
    /// are `Mwc::float` twice) and the MWC after must be equal bit for bit. A case whose rounded weights sum to under the
    /// first draw makes the original read the stack past the last triangle (`random_point_k` flags it): skipped.
    #[test]
    fn random_point_matches_the_original() {
        let u = |t: &str| u32::from_str_radix(t, 16).unwrap();
        let (mut n, mut off) = (0, 0);
        for line in include_str!("testdata/random_point_ref.txt").lines() {
            let line = line.replace('_', "");
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = i.split_whitespace().collect();
            let k: usize = w[0].parse().unwrap();
            let v = |at: usize| Vec3::new(f32::from_bits(u(w[at])), f32::from_bits(u(w[at + 1])), f32::from_bits(u(w[at + 2])));
            let mut q = quad();
            q.nv = k as u8;
            for c in 0..k {
                q.verts[c] = v(1 + 3 * c);
            }
            let mut mwc = crate::vehicles::parked::Mwc { lo: u(w[1 + 3 * k]), carry: u(w[2 + 3 * k]) };
            let (r1, r2) = (mwc.float(), mwc.float());
            let (p, past) = random_point_k(&q, r1, r2);
            let want: Vec<u32> = o.split_whitespace().map(u).collect();
            assert_eq!((mwc.lo, mwc.carry), (want[3], want[4]), "{line}");
            n += 1;
            if past {
                off += 1;
                continue;
            }
            assert_eq!([p.x.to_bits(), p.y.to_bits(), p.z.to_bits()], want[..3], "{line}");
        }
        assert_eq!(n, 6000);
        assert!(off < 10, "{off} cases past the last triangle");
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
        // F-84843 run offline on the original's code (tools/stop_time_reference.py -> testdata/stop_time_ref.txt): 4,000
        // cases of idle clip length (0, short, 5 and over, negative, tiny, huge, NaN) with the walker's MWC state and the
        // M flags; the timer, the MWC after and the stop count (+1 in bits 3 and up, bits 0x4 / 0x2 cleared: p.stops += 1)
        // must be equal bit for bit.
        let u = |t: &str| u32::from_str_radix(t, 16).unwrap();
        let mut n = 0;
        for line in include_str!("testdata/stop_time_ref.txt").lines() {
            let line = line.replace('_', "");
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = i.split_whitespace().map(u).collect();
            let r: Vec<u32> = o.split_whitespace().map(u).collect();
            let mut mwc = crate::vehicles::parked::Mwc { lo: w[1], carry: w[2] };
            let t = stop_time(f32::from_bits(w[0]), &mut mwc);
            assert_eq!((t.to_bits(), mwc.lo, mwc.carry), (r[0], r[1], r[2]), "{line}");
            assert_eq!((w[3] & !6) + 8, r[3], "{line}");
            n += 1;
        }
        assert_eq!(n, 4000);
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

    /// F-70932 with the grid's cell code (F-88455, F-99153) run whole offline
    /// (tools/people_room_reference.py -> testdata/people_room_ref.txt): 4,000 cases - grids of 1 .. 8 x 1 .. 8 cells,
    /// the point on and off the grid and on cell edges, up to 24 objects of kinds 0 .. 7 a float step or two about
    /// either spacing's edge, same and other types. The answer equal in every case.
    #[test]
    fn room_matches_original() {
        let txt = include_str!("testdata/people_room_ref.txt");
        let hex = |s: &str| u32::from_str_radix(s, 16).unwrap();
        let f = |s: &str| f32::from_bits(hex(s));
        let (mut cases, mut bad, mut blocked) = (0, 0, 0);
        for l in txt.lines().filter(|l| !l.is_empty()) {
            let (head, rest) = l.split_once(" | ").unwrap();
            let (objs, want) = rest.rsplit_once(" -> ").unwrap();
            let h: Vec<&str> = head.split_whitespace().collect();
            let grid = crate::vehicles::traffic::CrossGrid { min: [f(h[2]), f(h[3])], n: [hex(h[0]) as i32, hex(h[1]) as i32], bounded: true };
            let p = Vec3::new(f(h[4]), f(h[5]), f(h[6]));
            let o: Vec<&str> = objs.split_whitespace().collect();
            let list: Vec<RoomObj> = o.chunks(5).map(|c| RoomObj { kind: hex(c[0]) as u8, pos: Vec3::new(f(c[1]), f(c[2]), f(c[3])), same_type: c[4] == "1" }).collect();
            assert_eq!(list.len(), hex(h[9]) as usize);
            let got = room_blocked(&grid, p, &list, f(h[7]), f(h[8]));
            cases += 1;
            blocked += got as usize;
            if got != (want.trim() == "1") {
                bad += 1;
                if bad <= 5 {
                    eprintln!("room mismatch: {l} -> port {got}");
                }
            }
        }
        eprintln!("people room: {cases} cases ({blocked} no room), {bad} mismatches");
        assert!(cases >= 4000);
        assert_eq!(bad, 0);
    }

    /// One walker update of a recording of the running original (work/traffic-detail/fix37/tools/mkcases.py): the world's
    /// answers in the order the original asked, every query the port makes checked against them.
    struct ReplayWorld {
        ins: Option<bool>,
        sw: Vec<Vec<u32>>,
        gr: Vec<Vec<u32>>,
        ln: Vec<Vec<u32>>,
        mv: Vec<Vec<u32>>,
        ent: u32,
        isw: usize,
        igr: usize,
        iln: usize,
        bad: Vec<String>,
        say: Option<u32>,
    }

    fn rv(c: &[u32], i: usize) -> Vec3 {
        Vec3::new(f32::from_bits(c[i]), f32::from_bits(c[i + 1]), f32::from_bits(c[i + 2]))
    }

    fn same(a: Vec3, c: &[u32], i: usize) -> bool {
        a.x.to_bits() == c[i] && a.y.to_bits() == c[i + 1] && a.z.to_bits() == c[i + 2]
    }

    impl DodgeWorld for ReplayWorld {
        fn inside(&mut self, _c: Vec3, _r: f32) -> bool {
            match self.ins.take() {
                Some(v) => v,
                None => {
                    self.bad.push("inside: not asked in the original".into());
                    false
                }
            }
        }
        fn sweep(&mut self, a: Vec3, b: Vec3, r: f32, car: bool) -> Option<(Vec3, u32)> {
            let Some(s) = self.sw.get(self.isw).cloned() else {
                self.bad.push(format!("sweep {}: not asked in the original", self.isw));
                return None;
            };
            self.isw += 1;
            if !(same(a, &s, 0) && same(b, &s, 3) && r.to_bits() == s[6]) {
                self.bad.push(format!("sweep {} {a} -> {b} r {r} (car {car}), original {} -> {} r {}", self.isw - 1, rv(&s, 0), rv(&s, 3), f32::from_bits(s[6])));
            }
            (s[7] != 0).then(|| (rv(&s, 8), s[11]))
        }
        fn ground(&mut self, a: Vec3, b: Vec3) -> bool {
            let Some(g) = self.gr.get(self.igr).cloned() else {
                self.bad.push(format!("ground {}: not asked in the original", self.igr));
                return false;
            };
            self.igr += 1;
            if !(same(a, &g, 0) && same(b, &g, 3)) {
                self.bad.push(format!("ground {} {a} -> {b}, original {} -> {}", self.igr - 1, rv(&g, 0), rv(&g, 3)));
            }
            g[6] != 0
        }
        fn lane(&mut self, x: Vec3, r: f32) -> Option<(Vec3, f32)> {
            let Some(l) = self.ln.get(self.iln).cloned() else {
                self.bad.push(format!("lane {}: not asked in the original", self.iln));
                return None;
            };
            self.iln += 1;
            if !(same(x, &l, 0) && r.to_bits() == l[3]) {
                self.bad.push(format!("lane {} at {x} r {r}, original {} r {}", self.iln - 1, rv(&l, 0), f32::from_bits(l[3])));
            }
            (l[4] & 0xFF != 0).then(|| (rv(&l, 5), f32::from_bits(l[8])))
        }
        fn mover(&mut self, clip: usize, t: f32) -> Vec3 {
            let e = self.ent + 20 * clip as u32;
            // the sample of that entry at that time (F-37443's: at the floored time, site 0, and at the clip's end, site 1;
            // the re path's, site 2)
            // (site 2 records T, not the time + T the re path asks at: one per update)
            let hit = self.mv.iter().find(|m| m[1] == e && ((m[0] == 0 && f32::from_bits(m[2]) == t) || m[0] == 2));
            let end = self.mv.iter().find(|m| m[1] == e && m[0] == 1);
            match (hit, end) {
                (Some(m), _) => rv(m, 3),
                (None, Some(m)) => rv(m, 3),
                _ => {
                    self.bad.push(format!("mover {clip} at {t}: not sampled in the original"));
                    Vec3::ZERO
                }
            }
        }
        fn stop(&mut self) {}
        fn speech(&mut self, kind: u32) {
            self.say = Some(kind);
        }
        fn drop_rails(&mut self) {}
    }

    /// The avoidance part's words (M as u32 words) the dodge writes, as the port holds them; `cars` maps the threat index
    /// to the recorded object, `ent` the avoid list entries to their addresses.
    fn avoid_words(m: &Avoid, cars: &[u32], ent: u32) -> Vec<(usize, u32)> {
        let mut o = Vec::new();
        for (k, p) in m.pts.iter().enumerate() {
            o.extend([(4 * k, p.x.to_bits()), (4 * k + 1, p.y.to_bits()), (4 * k + 2, p.z.to_bits())]);
        }
        o.push((20, m.car.map_or(0, |j| cars.get(j).copied().unwrap_or(0xFFFF_FFFF))));
        for (w, v) in [(24, m.stop_at), (32, m.dir), (40, m.from), (44, m.to)] {
            o.extend([(w, v.x.to_bits()), (w + 1, v.y.to_bits()), (w + 2, v.z.to_bits())]);
        }
        o.extend([(59, m.t.to_bits()), (60, m.time.to_bits()), (61, m.len.to_bits()), (62, m.clip.map_or(0, |e| ent + 20 * e as u32))]);
        o.extend([(63, m.yaw.to_bits()), (64, m.stretch.to_bits()), (65, m.flags & 6)]);
        o
    }

    fn avoid_of(w: &[u32], cars: &[u32], ent: u32) -> Avoid {
        let f = |i: usize| f32::from_bits(w[i]);
        Avoid {
            pts: [rv(w, 0), rv(w, 4), rv(w, 8), rv(w, 12), rv(w, 16)],
            car: (w[20] != 0).then(|| cars.iter().position(|&o| o == w[20]).unwrap_or(usize::MAX)),
            stop_at: rv(w, 24),
            dir: rv(w, 32),
            from: rv(w, 40),
            to: rv(w, 44),
            t: f(59),
            time: f(60),
            len: f(61),
            clip: (w[62] != 0).then(|| ((w[62] - ent) / 20) as usize),
            yaw: f(63),
            stretch: f(64),
            flags: w[65],
        }
    }

    fn word_name(w: usize) -> String {
        match w {
            0..=19 => format!("P{}.{}", w / 4, w % 4),
            20 => "car".into(),
            24..=26 => format!("stop_at.{}", w - 24),
            32..=34 => format!("dir.{}", w - 32),
            40..=42 => format!("from.{}", w - 40),
            44..=46 => format!("to.{}", w - 44),
            59 => "T".into(),
            60 => "time".into(),
            61 => "len".into(),
            62 => "clip".into(),
            63 => "yaw".into(),
            64 => "stretch".into(),
            65 => "flags".into(),
            _ => format!("w{w}"),
        }
    }

    /// The car dodge replayed on recordings of the running original (MCLA_DODGE_REPLAY = a case file of
    /// work/traffic-detail/fix37/tools/mkcases.py; docs/reference/people/car_dodge_rule.md "Evidence vs the running
    /// original"). (1) Each recorded walker update with a grid object past the moving test: the walker (P, r, U, rows, sex),
    /// the avoidance part before, the objects as the original read them, the world's answers in the original's order;
    /// `car_dodge` (F-15352 with F-94813 / F-78951 / F-50861 / F-83234 / F-37443) run on them; the words it writes compared
    /// with the original's after the call, its queries with the original's. (2) The walker run free: the port's own
    /// avoidance part carried from update to update with F-56160's step, against the recorded words before each update.
    #[test]
    #[ignore]
    fn car_dodge_replays_the_recording() {
        let Ok(path) = std::env::var("MCLA_DODGE_REPLAY") else { return };
        let text = std::fs::read_to_string(&path).unwrap();
        let h = |t: &str| u32::from_str_radix(t, 16).unwrap();
        let mut items: Vec<AvoidItem> = Vec::new();
        let mut durs: Vec<f32> = Vec::new();
        let mut ent = 0u32;
        let (mut cases, mut calls, mut equal, mut decided, mut dec_equal, mut q_bad) = (0, 0, 0, 0, 0, 0);
        let mut shown = 0;
        let mut free: HashMap<String, Avoid> = HashMap::new();
        let (mut free_n, mut free_eq, mut free_first, mut free_own) = (0, 0, Vec::<String>::new(), 0);
        let mut field_bad: HashMap<String, usize> = HashMap::new();
        let mut free_bad: HashMap<String, usize> = HashMap::new();
        let mut own_last: HashMap<String, bool> = HashMap::new();
        let mut free_eq_ys = 0;
        let mut free_side = 0;
        let mut free_fill = 0;
        let mut side_cmp: Vec<String> = Vec::new();
        let mut free_seen: std::collections::HashSet<String> = Default::default();
        let mut per_run: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
        for line in text.lines() {
            let mut kv: Vec<(&str, Vec<u32>)> = Vec::new();
            let mut it = line.split(' ');
            let kind = it.next().unwrap();
            let mut head = String::new();
            let mut run = String::new();
            let mut wkey = String::new();
            for t in it {
                let (k, v) = t.split_once('=').unwrap();
                match k {
                    "run" | "f" | "w" => {
                        head.push_str(&format!("{k}={v} "));
                        if k == "run" {
                            run = v.to_string();
                        }
                        if k == "w" {
                            wkey = v.to_string();
                        }
                    }
                    "e" => {
                        let es: Vec<Vec<u32>> = v.split(';').map(|e| e.split(',').map(h).collect()).collect();
                        items = es.iter().enumerate().map(|(i, e)| AvoidItem { ty: e[0], dir: e[1], clip: (e[2] != 0).then_some(i), audio: e[3] }).collect();
                        durs = es.iter().map(|e| f32::from_bits(e[4])).collect();
                    }
                    _ => kv.push((k, v.split(',').map(h).collect())),
                }
            }
            if kind == "avls" {
                ent = kv.iter().find(|(k, _)| *k == "ent").unwrap().1[0];
                continue;
            }
            let one = |k: &str| kv.iter().find(|(n, _)| *n == k).map(|x| x.1.clone());
            let all = |k: &str| kv.iter().filter(|(n, _)| *n == k).map(|x| x.1.clone()).collect::<Vec<_>>();
            cases += 1;
            let c = |k: &str| one(k).unwrap();
            let r = c("R");
            let w = DodgeWalker { p: rv(&c("P"), 0), r: f32::from_bits(c("r")[0]), u: rv(&c("U"), 0), rows: [rv(&r, 0), rv(&r, 3), rv(&r, 6)], female: c("sex")[0] == 1 };
            let dt = f32::from_bits(c("dt")[0]);
            let objs = all("car");
            let mut ptrs: Vec<u32> = objs.iter().map(|o| o[0]).collect();
            let cars: Vec<DodgeCar> = objs
                .iter()
                .map(|o| DodgeCar {
                    kind: o[1] as u8,
                    rail: false,
                    beh: true,
                    active: true,
                    a: rv(o, 2),
                    c: rv(o, 5),
                    q: rv(o, 8),
                    // (the recorder reads the velocity of a racer's car from its body; for any other object (kind 2, the
                    // ambient traffic) it has no known place, so the velocity the original used - v124 at the hook, flat -
                    // stands in; kind 2 has no acceleration term)
                    vel: if o[1] >= 2 { rv(o, 24) } else { rv(o, 11) },
                    prev: if o[1] >= 2 { rv(o, 24) } else { rv(o, 14) },
                    size: (f32::from_bits(o[17]), f32::from_bits(o[18])),
                    rv: f32::from_bits(o[19]),
                    burn: (o[20] != 0).then(|| f32::from_bits(o[21])),
                    inst: o[22] != 0,
                })
                .collect();
            let mi = c("Mi");
            let mo = c("Mo");
            // (a car the walker remembers - M+80 - that is not among this update's objects: an index past the objects, so
            // it maps back to its pointer)
            for w20 in [mi[20], mo[20]] {
                if w20 != 0 && !ptrs.contains(&w20) {
                    ptrs.push(w20);
                }
            }
            let slid = one("slid");
            let mwc = |k: usize| crate::vehicles::parked::Mwc { lo: slid.as_ref().map_or(0, |s| s[k]), carry: slid.as_ref().map_or(0, |s| s[k + 1]) };
            let mk = || ReplayWorld { ins: one("ins").map(|v| v[0] != 0), sw: all("sw"), gr: all("gr"), ln: all("ln"), mv: all("mv"), ent, isw: 0, igr: 0, iln: 0, bad: Vec::new(), say: None };
            let sp = one("sp").map(|v| v[0]);
            if !cars.is_empty() {
                let mut world = mk();
                let mut m = avoid_of(&mi, &ptrs, ent);
                let (mut speech, mut pick) = (mwc(11), mwc(13));
                calls += 1;
                car_dodge(&mut m, &w, &cars, dt, &items, &durs, &mut world, &mut speech, &mut pick);
                let dec = one("ins").is_some() || one("srch").is_some();
                decided += dec as usize;
                let mut diff = Vec::new();
                for (k, v) in avoid_words(&m, &ptrs, ent) {
                    let o = if k == 65 { mo[k] & 6 } else { mo[k] };
                    if o != v {
                        *field_bad.entry(word_name(k)).or_default() += 1;
                        diff.push(format!("{} port {:x} ({}) original {:x} ({})", word_name(k), v, f32::from_bits(v), o, f32::from_bits(o)));
                    }
                }
                if world.isw != world.sw.len() || world.igr != world.gr.len() || world.iln != world.ln.len() || world.ins.is_some() {
                    world.bad.push(format!("queries left: sweeps {}/{} grounds {}/{} lanes {}/{} inside {:?}", world.isw, world.sw.len(), world.igr, world.gr.len(), world.iln, world.ln.len(), world.ins));
                }
                if world.say != sp {
                    diff.push(format!("speech port {:?} original {:?}", world.say, sp));
                }
                q_bad += (!world.bad.is_empty()) as usize;
                let pr = per_run.entry(run.clone()).or_default();
                pr.0 += 1;
                pr.1 += (diff.is_empty() && world.bad.is_empty()) as usize;
                let pr = *pr;
                if diff.is_empty() && world.bad.is_empty() {
                    equal += 1;
                    dec_equal += dec as usize;
                } else if pr.0 - pr.1 <= 3 {
                    shown += 1;
                    eprintln!("DIFF {head}(decision {dec}): {} | {}", diff.join("; "), world.bad.join("; "));
                }
            }
            // (2) free run (resynced from the recording while a clip of the walker's own plays - M+260 bit 0x2, the turn to
            // the car after a dodge and the like, not the dodge's: F-94431 / the clip timer of F-97113)
            let rec_in = avoid_of(&mi, &ptrs, ent);
            // (and at the update after one, which then runs from the recording: the clip's end - F-73407 - is not modelled)
            let own = mi[65] & 2 != 0;
            let after_own = own_last.insert(format!("{run}/{wkey}"), own).unwrap_or(false);
            if own || after_own {
                free.insert(format!("{run}/{wkey}"), rec_in);
                free_own += 1;
                if own {
                    continue;
                }
            }
            let fm = free.entry(format!("{run}/{wkey}")).or_insert(rec_in);
            // (the bits other code sets - 0x1, 0x2 a clip of its own, 0x8 - from the recording; the port keeps 0x4)
            fm.flags = (mi[65] & !4) | (fm.flags & 4);
            free_n += (!after_own) as usize;
            let mut d = Vec::new();
            for ((k, a), (_, b)) in avoid_words(fm, &ptrs, ent).into_iter().zip(avoid_words(&rec_in, &ptrs, ent)) {
                if a != b && k > 20 && !after_own {
                    *free_bad.entry(word_name(k)).or_default() += 1;
                    d.push(format!("{} port {} original {}", word_name(k), f32::from_bits(a), f32::from_bits(b)));
                }
            }
            if !after_own && d.iter().all(|s| s.starts_with("yaw ") || s.starts_with("stretch ")) {
                free_eq_ys += 1;
            }
            if after_own {
            } else if d.is_empty() {
                free_eq += 1;
            } else if !d.iter().all(|s| s.starts_with("yaw ") || s.starts_with("stretch ")) && free_seen.insert(format!("{run}/{wkey}")) {
                free_first.push(format!("{head}: {}", d.join("; ")));
            }
            let mut world = mk();
            let (mut s2, mut p2) = (mwc(11), mwc(13));
            let trace = std::env::var("MCLA_DODGE_REPLAY_W").is_ok_and(|v| v == wkey);
            if trace {
                eprintln!("TRACE {head}in port fl {:x} t {} len {} T {} | orig fl {:x} t {} len {} T {} | cars {}", fm.flags, fm.time, fm.len, fm.t, mi[65], f32::from_bits(mi[60]), f32::from_bits(mi[61]), f32::from_bits(mi[59]), cars.len());
            }
            car_dodge(fm, &w, &cars, dt, &items, &durs, &mut world, &mut s2, &mut p2);
            if trace {
                eprintln!("TRACE {head}out port fl {:x} t {} len {} T {} | orig fl {:x} t {} len {} T {} | {:?}", fm.flags, fm.time, fm.len, fm.t, mo[65], f32::from_bits(mo[60]), f32::from_bits(mo[61]), f32::from_bits(mo[59]), world.bad);
            }
            // F-26402 (the sidestep, after the threat in F-97113, when not sliding and no clip of its own plays) on sphere 1's
            // slots from the port's fill: the other walkers' capsules at their recorded matrices of this frame (I+128 at their
            // F-15352 entry; nb=, work/traffic-detail/fix42/tools/mkcases42.py), the centre the walker's place (T's place of
            // the draw is not recorded). The draws are random (the walker's own MWC, not recorded), so the fill runs at the
            // updates whose sidestep the original recorded (a draw the frame before is then certain). A case file without
            // the neighbours (fix37's) replays the recorded slide start instead.
            if let Some(s) = one("side") {
                if fm.flags & 6 == 0 {
                    if one("me").is_some() {
                        let caps: Vec<FillCapsule> = all("nb")
                            .iter()
                            .enumerate()
                            .map(|(k, o)| FillCapsule::walker(k, [rv(o, 1), rv(o, 4), rv(o, 7)], rv(o, 10)))
                            .collect();
                        let hits = sphere_fill(w.p, AWARE_RADIUS, &caps, 0);
                        let slots = side_slots(&hits, |_| WALKER_TYPE);
                        let did = sidestep(fm, &w, vneg(w.rows[2]), &slots, &items, &durs, &mut world, &mut s2, &mut p2);
                        let (rd, rn) = (rv(&s, 1), rv(&s, 4));
                        let pd = fm.to - w.p;
                        let k = hits.iter().position(|h| h.is_some());
                        let n = k.and_then(|k| hits[k]).map_or(Vec3::ZERO, |h| h.n);
                        side_cmp.push(format!(
                            "{head}: slots {} sidestep {did} | push port {pd:.4} original {rd:.4} (|d| {:.4}) | n port {n:.4} original {rn:.4} (|d| {:.4})",
                            hits.iter().filter(|h| h.is_some()).count(),
                            if did { (pd - rd).length() } else { f32::NAN },
                            (n - rn).length()
                        ));
                        free_fill += did as usize;
                    } else {
                        slide_start(fm, &w, &items, &durs, None, rv(&s, 1), rv(&s, 4), f32::from_bits(s[7]), f32::from_bits(s[8]), true, &mut world, &mut s2, &mut p2);
                    }
                    free_side += 1;
                }
            }
            // F-56160 (the slide step) as dodge_frame does it
            if fm.flags & 4 != 0 {
                let first = fm.time == 0.0;
                fm.time += dt;
                if !first && fm.time >= fm.len {
                    fm.clip = None;
                    fm.flags &= !4;
                }
            }
            // the pose F-94626 while the walker's one-off clip plays (its state 3; here: an evade clip set): the yaw
            // correction M+252 taken off by at most the turn rate x dt, the stretch M+256 by its positive part x dt / max(T, dt)
            if fm.clip.is_some() {
                let (yaw, stretch) = clip_correction(fm.yaw, fm.stretch, fm.t, dt);
                fm.yaw = yaw.0;
                fm.stretch = stretch.0;
            }
        }
        let mut fb: Vec<_> = field_bad.into_iter().collect();
        fb.sort();
        eprintln!("replay {path}: {cases} walker updates, {calls} with objects past the moving test: {equal} equal (words written + queries + speech), {q_bad} with a query difference; {decided} with a decision (inside / search), {dec_equal} of them equal; words differing: {fb:?}");
        eprintln!("per run (updates with objects, equal): {per_run:?}");
        eprintln!("free run: {free_n} updates compared, {free_eq} with the avoidance part equal before the update; {free_own} updates during the walker's own clips (resynced)");
        let mut fb: Vec<_> = free_bad.into_iter().collect();
        fb.sort();
        eprintln!("free run, words differing: {fb:?}; {free_eq_ys} equal but for yaw / stretch");
        eprintln!("free run: {free_side} recorded sidesteps reached with the flags clear, {free_fill} started by the port from its fill");
        for l in side_cmp {
            eprintln!("SIDE {l}");
        }
        for l in free_first {
            eprintln!("FREE {l}");
        }
    }
}
