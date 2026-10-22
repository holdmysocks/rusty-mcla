//! Loose props as the car's hull meets them: what the original does when an active body
//! reaches an INACTIVE instance of the physics level (work/veh/gjkfix/z4/z4_decode.md, the
//! decodes in work/veh/gjkfix/p1/ and work/veh/gjk_exact.md part 10). The prop has no body when
//! it is hit: the tick's velocity solve takes it for immovable, the breaking pass gives the car
//! only a share of that impulse, turns the prop - or the group of it that broke off - into a
//! body, and solves again.
//!
//! This file holds the props' data as the physics reads it (the fragment types and where they
//! stand), the cache entries and pieces that have come loose, the narrow phase between a car's
//! hull and a prop's parts, the breaking pass, and the pieces' own motion.

use super::collision::{surface_touch, Surface};
use super::exact::{self as ex, cross, dot3, fma, recip};
use super::gjk::{convex_contact, HullCore, Nearest, PartCore, Support};
use super::hull::HullShape;
use super::solver::{self, Body, Pushed, Side, Touch};
use glam::Vec3;
use mcla_formats::rage::fragphys::{FragBound, FragInstance, FragType};
use std::collections::HashMap;
use std::sync::Arc;

/// The edge of the cells the instances are looked up by (m).
const CELL: f32 = 16.0;

/// Every prop of the level: the types and the instances, as the file has them.
#[derive(Debug, Default)]
pub struct LooseWorld {
    pub types: Vec<FragType>,
    /// each instance and the number of its type in `types`
    pub instances: Vec<(FragInstance, usize)>,
    cells: HashMap<(i32, i32), Vec<u32>>,
}

fn cell_of(x: f32, z: f32) -> (i32, i32) {
    ((x / CELL).floor() as i32, (z / CELL).floor() as i32)
}

pub fn v(a: [f32; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

fn m4(a: &[[f32; 3]; 4]) -> [Vec3; 4] {
    [v(a[0]), v(a[1]), v(a[2]), v(a[3])]
}

/// A point of a frame carried into the world (a sum of four, vmsum4fp with the column).
pub fn xf(m: &[Vec3; 4], p: Vec3) -> Vec3 {
    Vec3::new((m[3].x + m[2].x * p.z) + (m[1].x * p.y + m[0].x * p.x), (m[3].y + m[2].y * p.z) + (m[1].y * p.y + m[0].y * p.x), (m[3].z + m[2].z * p.z) + (m[1].z * p.y + m[0].z * p.x))
}

/// A world vector in a frame's axes (three sums of three).
pub fn inv(m: &[Vec3; 4], d: Vec3) -> Vec3 {
    Vec3::new(dot3(m[0], d), dot3(m[1], d), dot3(m[2], d))
}

impl LooseWorld {
    pub fn new(types: Vec<FragType>, instances: Vec<FragInstance>) -> LooseWorld {
        let by_off: HashMap<usize, usize> = types.iter().enumerate().map(|(i, t)| (t.off, i)).collect();
        let mut out = LooseWorld { types, instances: Vec::new(), cells: HashMap::new() };
        // (the torn bumpers' own type, made by the game's code rather than read from the file)
        out.types.push(piece_type());
        for inst in instances {
            let Some(&t) = by_off.get(&inst.frag) else { continue };
            // (state 3 is never put into the level: the glow cards)
            if inst.state == 3 || !inst.matrix[3].iter().all(|x| x.is_finite()) {
                continue;
            }
            let i = out.instances.len() as u32;
            out.cells.entry(cell_of(inst.matrix[3][0], inst.matrix[3][2])).or_default().push(i);
            out.instances.push((inst, t));
        }
        out
    }

    /// The prop file of a data directory.
    pub fn load(file: &std::path::Path) -> Option<Arc<LooseWorld>> {
        let res = crate::assets::load_resource_file(file).ok()?;
        let types = mcla_formats::rage::fragphys::parse_frag_types(&res);
        let instances = mcla_formats::rage::fragphys::parse_frag_instances(&res);
        if std::env::var_os("MCLA_DBG_LOOSELIST").is_some() {
            for (n, i) in instances.iter().enumerate() {
                eprintln!("LOOSEINST {n} state {} frag {:x} at {:?}", i.state, i.frag, i.matrix[3]);
            }
        }
        Some(Arc::new(LooseWorld::new(types, instances)))
    }

    /// The instances whose place is within `r` of `p` on the map, in the file's order.
    pub fn near(&self, p: Vec3, r: f32) -> Vec<u32> {
        let (x0, z0) = cell_of(p.x - r, p.z - r);
        let (x1, z1) = cell_of(p.x + r, p.z + r);
        let mut out = Vec::new();
        for cx in x0..=x1 {
            for cz in z0..=z1 {
                if let Some(list) = self.cells.get(&(cx, cz)) {
                    for &i in list {
                        let m = &self.instances[i as usize].0.matrix[3];
                        if (m[0] - p.x).abs() <= r && (m[2] - p.z).abs() <= r {
                            out.push(i);
                        }
                    }
                }
            }
        }
        out.sort_unstable();
        out
    }
}

// ---------------------------------------------------------------------------------------------
// Cache entries: which groups of a prop an instance still holds, and what it weighs
// (0x824A9130, 0x8259A580, 0x8259A390, 0x8258E858 / 0x825A4088; p1/break_decode.md 3.3, 3.4).

/// A fragment cache entry: the groups it has lost, its age, and its mass properties.
#[derive(Clone, Copy, Debug, Default)]
pub struct Entry {
    /// the groups NOT in it (+268)
    pub absent: u64,
    /// seconds since the prop first got an entry (+420)
    pub age: f32,
    /// the composite's centre of gravity (bound +80), the archetype's mass and moments
    pub cg: Vec3,
    pub mass: f32,
    pub inv_mass: f32,
    pub inertia: Vec3,
    pub inv_inertia: Vec3,
    /// (the port's own: the prop instance a piece broke off, for drawing it, props.rs `follow_level`)
    pub from: Option<u32>,
}

fn group_of(t: &FragType, child: usize) -> usize {
    t.children.get(child).map_or(0, |c| c.group as usize)
}

/// Is part `child` in an entry: the type has a bound there and its group has not left.
pub fn part_present(t: &FragType, absent: u64, child: usize) -> bool {
    t.parts.get(child).map_or(false, |p| p.bound.is_some()) && absent & (1 << group_of(t, child)) == 0
}

/// A group and everything that hangs on it, as a bit set.
pub fn subtree(t: &FragType, g: usize) -> u64 {
    let mut bits = 1u64 << g;
    let grp = &t.groups[g];
    for k in 0..grp.groups as usize {
        bits |= subtree(t, grp.first_group as usize + k);
    }
    bits
}

/// The mass of what an entry holds (0x824A6DC8 / 0x824A9458): the children's, in their order.
pub fn entry_mass(t: &FragType, absent: u64) -> f32 {
    let mut m = 0.0f32;
    for (c, child) in t.children.iter().enumerate() {
        if absent & (1 << group_of(t, c)) == 0 {
            m = child.mass + m;
        }
    }
    m
}

/// 0x824A9130: the entry's centre of gravity (by the parts' volumes; shifted by the type's +0x40
/// while the entry is younger than a quarter of a second), its mass, and its moments (each
/// part's own about its place, carried to the centre without its turn; no moment under a
/// twentieth of the largest).
pub fn entry_refresh(t: &FragType, e: &mut Entry, mass: f32) {
    let mut acc = Vec3::ZERO;
    let mut tot = 0.0f32;
    for (c, part) in t.parts.iter().enumerate() {
        let Some(b) = part.bound.as_ref().filter(|_| part_present(t, e.absent, c)) else { continue };
        let w = b.volume;
        tot = w + tot;
        let m = m4(&part.matrix);
        let p = if b.flag6 { xf(&m, v(b.cg)) } else { m[3] };
        acc = p * w + acc;
    }
    if tot > f32::from_bits(0x3586_37bd) {
        acc = acc * recip(tot);
    }
    e.cg = acc;
    if e.age < 0.25 {
        e.cg = e.cg + v(t.cg_shift);
    }
    e.mass = mass;
    e.inv_mass = if mass == 0.0 { 0.0 } else { 1.0 / mass };
    let mut sum = 0.0f32;
    let mut inertia = Vec3::ZERO;
    for (c, part) in t.parts.iter().enumerate() {
        let Some(b) = part.bound.as_ref().filter(|_| part_present(t, e.absent, c)) else { continue };
        let (m, own) = (t.children[c].mass, v(t.children[c].inertia));
        let d = (v(part.matrix[3]) + v(b.cg)) - e.cg;
        sum = m + sum;
        let (zz, yy, xx) = (d.z * d.z, d.y * d.y, d.x * d.x);
        inertia = inertia + Vec3::new(fma(yy + zz, m, own.x), fma(xx + zz, m, own.y), fma(xx + yy, m, own.z));
    }
    if sum > f32::MIN_POSITIVE {
        // (0x825A4088)
        let mut i = inertia.to_array();
        let big = if i[0] >= i[1] { if i[0] >= i[2] { 0 } else { 2 } } else if i[1] >= i[2] { 1 } else { 2 };
        for j in 0..3 {
            if j != big && i[j] * 20.0 < i[big] {
                i[j] = i[big] * 0.05;
            }
        }
        e.inertia = Vec3::from_array(i);
        e.inv_inertia = Vec3::new(if i[0] == 0.0 { f32::MAX } else { recip(i[0]) }, if i[1] == 0.0 { f32::MAX } else { recip(i[1]) }, if i[2] == 0.0 { f32::MAX } else { recip(i[2]) });
    }
}

// ---------------------------------------------------------------------------------------------
// Bodies of what has come loose (the simulator's pool bodies, 672 bytes; p1/body_decode.md).

/// The body of an activated prop or piece.
#[derive(Clone, Copy, Debug)]
pub struct PBody {
    /// +256 and +208..: the centre of gravity in the world and the rows
    pub at: Vec3,
    pub rows: [Vec3; 3],
    pub v: Vec3,
    pub w: Vec3,
    /// the momenta (+304, +320)
    pub p: Vec3,
    pub l: Vec3,
    pub force: Vec3,
    pub torque: Vec3,
    /// the impulses not yet in the momenta (+368, +384)
    pub jf: Vec3,
    pub jt: Vec3,
    /// what the push solve asks (+400, +416)
    pub push: Vec3,
    pub turn: Vec3,
    /// the instance's matrix as the last collide is to have it (+96), and whether it is set (+660)
    pub last: [Vec3; 4],
    /// the instance's matrix before this tick's move (+32)
    pub prev: [Vec3; 4],
    pub last_set: bool,
    pub deep: bool,
    pub deep_count: u8,
    pub mass: f32,
    pub inv_mass: f32,
    pub inertia: Vec3,
    pub inv_inertia: Vec3,
    pub max_speed: f32,
    pub max_spin: f32,
    /// moves since the rows were made square (+664)
    pub moves: u32,
}

/// 0x8259BCC8 / 0x8259BD78: a velocity and its momentum cut back to the body's limit.
fn capped(v: Vec3, p: Vec3, max: f32) -> (Vec3, Vec3) {
    let l2 = fma(v.y, v.y, fma(v.z, v.z, v.x * v.x));
    if l2 <= max * max {
        return (v, p);
    }
    let mut e = (1.0 / (l2 as f64).sqrt()) as f32;
    for _ in 0..2 {
        e = fma(ex::fnms(e * e, l2, 1.0), e * 0.5, e);
    }
    let k = e * max;
    (v * k, p * k)
}

impl PBody {
    /// 0x8259E648 on an instance at `matrix` whose bound's centre of gravity is `cg`.
    fn new(matrix: &[Vec3; 4], cg: Vec3, e: &Entry, max_speed: f32, max_spin: f32) -> PBody {
        PBody {
            at: xf(matrix, cg),
            rows: [matrix[0], matrix[1], matrix[2]],
            v: Vec3::ZERO,
            w: Vec3::ZERO,
            p: Vec3::ZERO,
            l: Vec3::ZERO,
            force: Vec3::ZERO,
            torque: Vec3::ZERO,
            jf: Vec3::ZERO,
            jt: Vec3::ZERO,
            push: Vec3::ZERO,
            turn: Vec3::ZERO,
            last: *matrix,
            prev: *matrix,
            last_set: false,
            deep: false,
            deep_count: 0,
            mass: e.mass,
            inv_mass: e.inv_mass,
            inertia: e.inertia,
            inv_inertia: e.inv_inertia,
            max_speed,
            max_spin,
            moves: 0,
        }
    }

    /// The angular momentum of a turning rate (0x8259E180).
    fn momentum_of(&self, w: Vec3) -> Vec3 {
        let iw = ex::world_inertia(&self.rows, self.inertia);
        Vec3::new(dot3(Vec3::new(iw[0].x, iw[1].x, iw[2].x), w), dot3(Vec3::new(iw[0].y, iw[1].y, iw[2].y), w), dot3(Vec3::new(iw[0].z, iw[1].z, iw[2].z), w))
    }

    /// 0x8259DB98: an impulse at a point, taken into the momenta at once.
    pub fn impulse_now(&mut self, vec: Vec3, point: Vec3) {
        self.p = self.p + vec;
        (self.v, self.p) = capped(self.p * self.inv_mass, self.p, self.max_speed);
        let r = point - self.at;
        self.l = self.l + cross(r, vec);
        (self.w, self.l) = capped(ex::spin_of(self.l, &self.rows, self.inv_inertia), self.l, self.max_spin);
    }

    pub fn as_solver(&self) -> Body {
        Body { at: self.at, rows: self.rows, vel: self.v, ang: self.w, inv_mass: self.inv_mass, inv_moments: self.inv_inertia, force: self.force, torque: self.torque }
    }
}

// ---------------------------------------------------------------------------------------------
// The objects of the level that a car has had to do with.

/// A prop instance or a piece broken off one.
#[derive(Clone, Debug)]
pub struct Obj {
    pub ty: usize,
    /// the prop instance this is (None: a piece, a cache entry's own instance)
    pub inst: Option<u32>,
    /// the instance's matrix (inst +16)
    pub matrix: [Vec3; 4],
    pub entry: Option<Entry>,
    /// inst +80 bit 1: it has been moved or broken by the breaking pass
    pub hit: bool,
    pub body: Option<PBody>,
    /// its place among the level's indices and among the instances' addresses, against the
    /// car's (0): who is A in a pair, and whose contacts an island's rows begin with
    pub level: i64,
    pub addr: i64,
    /// (stage R) another vehicle: its bound and what its archetype holds
    pub veh: Option<VehObj>,
    /// its body's sleep object (body_decode.md 6.1; a torn bumper's is the piece's own)
    pub sleep: Sleep,
    /// taken out of the simulator asleep (0x82588D90): inactive at its last matrix until a contact wakes it (`wake_obj`)
    pub dormant: bool,
    /// an ambient (traffic) car of the session (`Amb`)
    pub amb: Option<Amb>,
}

/// Who a contact's side is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    Car,
    Obj(usize),
    /// the city (always side B)
    City,
}

/// A contact of a pair of parts as the manifold keeps it.
#[derive(Clone, Copy, Debug)]
pub struct Slot2 {
    pub t: Touch,
    pub age: u16,
    pub enabled: bool,
    /// which triangle of the car's hull (for its material)
    pub element: u16,
    /// what the city's polygon is made of
    pub surface: Surface,
    /// what side A is made of, where A is an ambient car (its triangle's material)
    pub surface_a: Surface,
}

/// The contacts of one part of an object with one part of the car (a sub-manifold).
#[derive(Clone, Debug)]
pub struct Mani {
    pub a: Who,
    pub b: Who,
    pub part_a: u16,
    pub part_b: u16,
    /// the city's cell (side B the city)
    pub cell: usize,
    /// its parts' instance broke this tick: the pair it belonged to is no more
    pub stale: bool,
    /// the tick's collide has looked at this pair of parts
    pub visited: bool,
    /// the island the tick's collide put it in (the car's: 0), and its pair's place in the island's tree
    pub island: usize,
    pub order: usize,
    pub slots: Vec<Slot2>,
    /// a constraint's manifold (0x82591210): its one contact made afresh every update
    pub rope: bool,
}

impl Mani {
    /// A new contact into the manifold (0x825C65E8 -> 0x825A37A8, 0x825A3028 / 0x825A3838): it
    /// takes the place of the kept one within 2 cm of it, by A's point, that a refresh has seen -
    /// and its multipliers, and its element if that one was the deeper - or is added.
    fn insert(&mut self, fresh: Touch, element: u16, surface: Surface, surface_a: Surface) {
        let mut nearest = (f32::from_bits(0x39d1_b717), None);
        for (i, s) in self.slots.iter().enumerate() {
            let d = s.t.a.local - fresh.a.local;
            let d2 = fma(d.x, d.x, fma(d.z, d.z, d.y * d.y));
            if d2 < nearest.0 && s.age != 0 {
                nearest = (d2, Some(i));
            }
        }
        let slot = Slot2 { t: fresh, age: 0, enabled: true, element, surface, surface_a };
        match nearest.1 {
            Some(i) => {
                let old = self.slots[i];
                self.slots[i] = Slot2 { t: Touch { kept: old.t.kept, pushed: old.t.pushed, ..fresh }, element: if old.t.depth >= fresh.depth { old.element } else { element }, surface: if old.t.depth >= fresh.depth { old.surface } else { surface }, surface_a: if old.t.depth >= fresh.depth { old.surface_a } else { surface_a }, ..slot };
            }
            None if self.slots.len() < 6 => self.slots.push(slot),
            // (the seventh, 0x825A32C0)
            None => {
                let k = super::hull::seventh_contact(std::array::from_fn(|i| self.slots[i].t.a.local), fresh.a.local);
                if k < 6 && !super::vehsim::dbg_flag("MCLA_DBG_RA3") {
                    self.slots[k] = slot;
                }
            }
        }
    }
}

/// A part ready for the detector.
struct Shape<'a> {
    core: Core<'a>,
    margin: f32,
    cg: Vec3,
    bbox: (Vec3, Vec3),
    now: [Vec3; 4],
    last: [Vec3; 4],
}

/// The two parts' boxes, each grown to hold itself at its last matrix too, tested along their
/// six face axes (0x825CCE00's cull; a decision only, written plainly).
fn boxes_meet(a: &Shape, b: &Shape) -> bool {
    let swept = |s: &Shape| {
        let (c, h) = ((s.bbox.1 + s.bbox.0) * 0.5, (s.bbox.1 - s.bbox.0) * 0.5);
        let (m, l) = (&s.now, &s.last);
        let r: [Vec3; 3] = std::array::from_fn(|i| inv(m, l[i]));
        let t = inv(m, l[3] - m[3]);
        let c2 = t + r[0] * c.x + r[1] * c.y + r[2] * c.z;
        let e2 = r[0].abs() * h.x + r[1].abs() * h.y + r[2].abs() * h.z;
        let (hi, lo) = ((c + h).max(c2 + e2), (c - h).min(c2 - e2));
        let mid = (hi + lo) * 0.5;
        (xf(m, mid), hi - mid)
    };
    let ((wa, ha), (wb, hb)) = (swept(a), swept(b));
    let eps = Vec3::splat(1e-6);
    let r: [Vec3; 3] = std::array::from_fn(|i| inv(&b.now, a.now[i]));
    let ab: [Vec3; 3] = std::array::from_fn(|i| r[i].abs() + eps);
    let t = inv(&b.now, wa - wb);
    let q = -Vec3::new(r[0].dot(t), r[1].dot(t), r[2].dot(t));
    let w = r[0] * q.x + r[1] * q.y + r[2] * q.z;
    let col = |k: usize| Vec3::new(ab[0][k], ab[1][k], ab[2][k]);
    (0..3).all(|i| ha[i] + ab[i].dot(hb) > q[i].abs()) && (0..3).all(|k| hb[k] + col(k).dot(ha) > w[k].abs())
}

/// What the car is, for the props' side of a tick.
pub struct CarIn<'a> {
    pub hull: &'a HullShape,
    /// the hull bound's centre of gravity (bound +0x50) and margin
    pub hull_cg: Vec3,
    pub hull_margin: f32,
    /// the car's matrix as the collide has it, and as its last one (body +96)
    pub now: [Vec3; 4],
    pub last: [Vec3; 4],
    /// chassis +224
    pub speed: f32,
    /// the composite bound's box (min, max)
    pub bound: (Vec3, Vec3),
    /// the wheels' parts (the composite's parts 1..), None where there is none
    pub wheels: Vec<Option<WheelIn>>,
    /// (stage R, agent D) the body's velocity as the tick came in (collider +272 at the callbacks), BoundFriction /
    /// BoundElasticity, and chassis +592 bits 0x10 / 0x04
    pub vel: Vec3,
    pub touch: (f32, f32),
    pub wiped: bool,
}

/// A wheel's part of the car's bound for the props' side of a tick: its core where it stands, its matrix
/// (part x instance) now and as the last collide is to have had it - the car's last matrix with the wheel
/// at the top of its travel - and its box's half sizes.
#[derive(Clone, Copy)]
pub struct WheelIn {
    pub core: super::gjk::WheelCore,
    pub now: [Vec3; 4],
    pub last: [Vec3; 4],
    pub half: Vec3,
}

/// The car's momenta while the breaking pass hands out its shares.
pub struct CarDyn<'a> {
    pub p: &'a mut Vec3,
    pub l: &'a mut Vec3,
    pub mass: f32,
    pub inverse: Vec3,
    /// the rows the turning rate is taken through (+208..)
    pub rows: [Vec3; 3],
}

/// Everything loose a car has met.
#[derive(Clone, Debug, Default)]
pub struct LooseState {
    pub objs: Vec<Obj>,
    by_inst: HashMap<u32, usize>,
    pub manis: Vec<Mani>,
    pieces_made: usize,
    /// (debug) ticks
    pub tick: u32,
    /// the cache entries made so far: each took the next instance of the pool (0x90 bytes, downwards)
    entries_made: i64,
    /// the level indices handed out in the update `.0` so far
    indices_taken: (i64, usize),
    /// the place of the car's own pair with the city in its island's tree (None: no such pair this tick)
    car_city_order: Option<usize>,
    /// the car's torn-off bumpers in the level (`add_piece`)
    pub pieces: Vec<Piece>,
    /// (the game) one level shared by every car: the other cars are given as vehicles before each car's
    /// step (`set_vehicles`, not integrated here), and the loose bodies' pre-collide and end of tick run
    /// once a tick - with the first car's step and the last car's (`skip_pre`, `skip_end` for the others)
    pub live: bool,
    pub skip_pre: bool,
    pub skip_end: bool,
    /// (the game) a car after the tick's first: the loose bodies' own pairs are made already
    pub later: bool,
    /// the car is under MegaStealth (vehicles.rs `pu_ghost`, POWERUPS.md): it meets the map and nothing
    /// loose - its step leaves the level out
    pub ghost: bool,
    /// the constraints of the semis' cabs and trailers (sub_826E7340), in the update's list order
    pub ropes: Vec<Rope>,
    /// what the ambient cars' post-solve callback (slot 25, sub_826FAF98) adds to their render instance's
    /// four side sums (+560): (the car's uid, the side 0 .. 3, the amount) - vehsim.rs, taken by
    /// vehicles.rs after the steps
    pub amb_dmg: Vec<(u32, usize, f32)>,
    /// the effects the props' events start (propfx.rs, `prop_hit_fx_rule.md`): taken by the game after the steps
    pub fx_events: Vec<FxPlay>,
}

/// An effect a prop's event starts: its name and where it is played.
#[derive(Clone, Debug)]
pub struct FxPlay {
    pub effect: String,
    pub at: FxAt,
    /// the effect instance's velocity (the deflection pass gives the car's; every other event 0)
    pub vel: Vec3,
}

/// Where an event effect is placed (`sub_82736D38`).
#[derive(Clone, Copy, Debug)]
pub enum FxAt {
    /// at a world point, unrotated (the position parameter: impact and deflection events)
    World(Vec3),
    /// on a matrix it follows each frame (the matrix-pointer parameter: the break events): the object's
    /// matrix, or the matrix of one of its bones (`part`); the offset in that frame
    Follow { obj: usize, part: Option<u16>, offset: Vec3 },
}

/// What a run does not let one derive (the level's indices and the heap's addresses), given per
/// replay: `MCLA_LOOSE_HINTS` = words `b:<prop instance>` (the prop's level index is above the
/// car's: the car is A against it), `after:<prop instance>` (its address is above the car's),
/// `pb:<n>` / `pafter:<n>` the same for the n-th piece made.
fn hint(kind: &str, n: usize) -> bool {
    static H: std::sync::OnceLock<Vec<(String, usize)>> = std::sync::OnceLock::new();
    H.get_or_init(|| {
        std::env::var("MCLA_LOOSE_HINTS")
            .unwrap_or_default()
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter_map(|w| w.split_once(':').and_then(|(k, n)| n.parse().ok().map(|n| (k.to_string(), n))))
            .collect()
    })
    .iter()
    .any(|(k, i)| k == kind && *i == n)
}

/// `MCLA_LOOSE_HINTS` words `l<object>:<rank>` / `a<object>:<rank>`: the level index / the address of
/// the n-th object a replay comes to (props as the car nears them, pieces as they are made), as a
/// rank against the car's 0.
fn rank(kind: char, obj: usize) -> Option<i64> {
    static H: std::sync::OnceLock<Vec<(char, usize, i64)>> = std::sync::OnceLock::new();
    H.get_or_init(|| {
        std::env::var("MCLA_LOOSE_HINTS")
            .unwrap_or_default()
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter_map(|w| {
                let (k, v) = w.split_once(':')?;
                let kind = k.chars().next()?;
                Some((kind, k[1..].parse().ok()?, v.parse().ok()?))
            })
            .collect()
    })
    .iter()
    .find(|x| x.0 == kind && x.1 == obj)
    .map(|x| x.2)
}

/// The update of the recording a replay is at (set by the replay's loop; i64::MIN: none).
pub static REPLAY_K: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(i64::MIN);

/// What a stage-R recording logs of the session's physics level (`MCLA_LOOSE_LEVEL` = a file that
/// `veh_diff.py` writes from the side file): the car's level index and instance address, the
/// instance address of every prop it logged (by level index), the first address of the pool the
/// cache entries' instances come from, per update the head of the level's free index list, and the
/// city's own instance (the B of every contact with the city).
#[derive(Debug, Default)]
pub struct LevelHints {
    pub car: (i64, i64),
    pub addr: HashMap<i64, i64>,
    pub pool: Option<i64>,
    pub free: HashMap<i64, Vec<i64>>,
    pub city: Option<i64>,
}

pub fn level_hints() -> Option<&'static LevelHints> {
    static H: std::sync::OnceLock<Option<LevelHints>> = std::sync::OnceLock::new();
    H.get_or_init(|| {
        let text = std::fs::read_to_string(std::env::var_os("MCLA_LOOSE_LEVEL")?).ok()?;
        let mut h = LevelHints::default();
        let num = |w: &str| if let Some(x) = w.strip_prefix("0x") { i64::from_str_radix(x, 16).ok() } else { w.parse::<i64>().ok() };
        for line in text.lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            let n: Vec<i64> = w.iter().skip(1).filter_map(|x| num(x)).collect();
            match (w.first().copied(), n.len()) {
                (Some("car"), 2..) => h.car = (n[0], n[1]),
                (Some("obj"), 2..) => {
                    h.addr.insert(n[0], n[1]);
                }
                (Some("pool"), 1..) => h.pool = Some(n[0]),
                (Some("city"), 1..) => h.city = Some(n[0]),
                (Some("free"), 2..) => {
                    h.free.insert(n[0], n[1..].to_vec());
                }
                _ => {}
            }
        }
        Some(h)
    })
    .as_ref()
}

/// `MCLA_DBG_RA1`: the level indices and addresses by the older guesses, whatever the recording logs.
fn r1() -> bool {
    static D: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *D.get_or_init(|| std::env::var_os("MCLA_DBG_RA1").is_some())
}

fn dbg() -> bool {
    static D: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *D.get_or_init(|| std::env::var_os("MCLA_DBG_LOOSE").is_some())
}

/// A convex part with its matrix: what the detector is handed.
#[derive(Clone, Copy)]
pub enum Core<'a> {
    Part(PartCore),
    Hull(HullCore<'a>),
    Wheel(super::gjk::WheelCore),
}

impl Support for Core<'_> {
    fn support(&self, dir: Vec3) -> Vec3 {
        match self {
            Core::Part(p) => p.support(dir),
            Core::Hull(h) => h.support(dir),
            Core::Wheel(w) => w.support(dir),
        }
    }
    fn support_at(&self, dir: Vec3, by: Vec3, then: Vec3) -> Vec3 {
        match self {
            Core::Part(p) => p.support_at(dir, by, then),
            Core::Hull(h) => h.support_at(dir, by, then),
            Core::Wheel(w) => w.support_at(dir, by, then),
        }
    }
    fn support_id(&self, dir: Vec3, by: Vec3, then: Vec3) -> (Vec3, u32) {
        match self {
            Core::Part(p) => p.support_id(dir, by, then),
            Core::Hull(h) => h.support_id(dir, by, then),
            Core::Wheel(w) => w.support_id(dir, by, then),
        }
    }
}

impl<'a> Core<'a> {
    fn placed(&self, at: Vec3) -> Core<'a> {
        match *self {
            Core::Part(p) => Core::Part(PartCore { at, ..p }),
            Core::Hull(h) => Core::Hull(HullCore { at, ..h }),
            Core::Wheel(mut w) => {
                w.centre = at;
                Core::Wheel(w)
            }
        }
    }
}

/// A part of a prop's bound at a matrix: its core, its margin, its centre of gravity.
fn part_core(b: &FragBound, m: &[Vec3; 4]) -> (PartCore, f32) {
    let rows = [m[0], m[1], m[2]];
    match b.kind {
        0 => (PartCore { kind: 0, centroid: v(b.centroid), half: Vec3::ZERO, rows, at: m[3] }, b.round),
        1 => (PartCore { kind: 1, centroid: v(b.centroid), half: Vec3::splat(b.length), rows, at: m[3] }, b.round),
        _ => {
            // (0x825E03D4: half the box less the margin, each a rounded step)
            let half = (v(b.max) - v(b.min)) * 0.5 - Vec3::splat(b.margin);
            (PartCore { kind: 3, centroid: v(b.centroid), half, rows, at: m[3] }, b.margin)
        }
    }
}

/// What the swept test gives (0x825D9268): the touch's normal, the two points in the parts'
/// frames, how far through the tick it is, and the corners the hull's side ended on.
struct Swept {
    normal: Vec3,
    local_a: Vec3,
    local_b: Vec3,
    lambda: f32,
    last: Nearest,
}

/// The swept test of two convex parts (0x825D9268; p1/swept_decode.md): each is carried from
/// where its centre of gravity was to where it is, with the rows it has now; the detector is
/// asked again and again until the parts are 2 cm apart.
fn sweep(a: &Core, ma: f32, cga: Vec3, a_now: &[Vec3; 4], a_last: &[Vec3; 4], b: &Core, mb: f32, cgb: Vec3, b_now: &[Vec3; 4], b_last: &[Vec3; 4]) -> Option<Swept> {
    let da = xf(a_now, cga) - xf(a_last, cga);
    let db = xf(b_now, cgb) - xf(b_last, cgb);
    let rel = db - da;
    let (mut pa, mut pb) = (a_now[3] - da, b_now[3] - db);
    let mut dir = Vec3::Z;
    let mut r = convex_contact(&a.placed(pa), ma, &b.placed(pb), mb, &mut dir, true)?;
    let (mut lambda, mut last) = (0.0f32, 0.0f32);
    let tol = f32::from_bits(0x3a83_126f);
    let stop = f32::from_bits(0x3ca3_d70a);
    if r.distance > tol {
        let mut gap = r.distance;
        let mut steps = 0;
        loop {
            steps += 1;
            if steps > 32 {
                return None;
            }
            let proj = dot3(rel, r.normal) + 0.0;
            let step = gap * if proj == 0.0 { f32::MAX } else { recip(proj) };
            lambda = lambda + step;
            if 0.0 > step || lambda > 1.0 {
                return None;
            }
            if last >= lambda {
                break;
            }
            last = lambda;
            let k = 1.0 - lambda;
            pb = -(db * k - b_now[3]);
            pa = -(da * k - a_now[3]);
            r = convex_contact(&a.placed(pa), ma, &b.placed(pb), mb, &mut dir, true)?;
            if stop >= r.distance {
                break;
            }
            gap = r.distance - stop;
            if !(gap > tol) {
                break;
            }
        }
    }
    let at = |m: &[Vec3; 4], p: Vec3| [m[0], m[1], m[2], p];
    Some(Swept { normal: r.normal, local_a: inv(&at(a_now, pa), r.on_a - pa), local_b: inv(&at(b_now, pb), r.on_b - pb), lambda, last: r })
}

/// The swept test of a convex part against one city polygon (0x825CEE38's swept branch, 0x825D9268 inside): the
/// part's places now and last taken about the polygon's first corner `origin`, the mesh at rest at the origin
/// (its move is nil), the polygon `local` about that corner. The detector starts from (1, 0, 0) and keeps its
/// direction from one call to the next; the touch's normal, the part's point carried to its place now, the
/// polygon's point, and the distance between them along the normal - about `origin`, as the discrete detector
/// answers. (No "lambda below 1" test after it here, unlike the part pairs' 0x825CC930.)
fn sweep_polygon(core: &Core, margin: f32, cg: Vec3, now: &[Vec3; 4], last: &[Vec3; 4], local: &[Vec3; 3], face: Vec3, bisectors: [Vec3; 3], origin: Vec3) -> Option<Nearest> {
    let now = [now[0], now[1], now[2], now[3] - origin];
    let last = [last[0], last[1], last[2], last[3] - origin];
    let da = xf(&now, cg) - xf(&last, cg);
    let rel = Vec3::ZERO - da;
    let mut pa = now[3] - da;
    let mut dir = Vec3::X;
    let pm = super::hull::POLYGON_MARGIN;
    let mut r = super::gjk::contact_local(&core.placed(pa), Vec3::ZERO, local, face, bisectors, margin, pm, &mut dir)?;
    let (mut lambda, mut last_l) = (0.0f32, 0.0f32);
    let tol = f32::from_bits(0x3a83_126f);
    let stop = f32::from_bits(0x3ca3_d70a);
    if r.distance > tol {
        let mut gap = r.distance;
        let mut steps = 0;
        loop {
            steps += 1;
            if steps > 32 {
                return None;
            }
            let proj = dot3(rel, r.normal) + 0.0;
            let step = gap * if proj == 0.0 { f32::MAX } else { recip(proj) };
            lambda = lambda + step;
            if 0.0 > step || lambda > 1.0 {
                return None;
            }
            if last_l >= lambda {
                break;
            }
            last_l = lambda;
            pa = -(da * (1.0 - lambda) - now[3]);
            r = super::gjk::contact_local(&core.placed(pa), Vec3::ZERO, local, face, bisectors, margin, pm, &mut dir)?;
            if stop >= r.distance {
                break;
            }
            gap = r.distance - stop;
            if !(gap > tol) {
                break;
            }
        }
    }
    let la = inv(&[now[0], now[1], now[2], pa], r.on_a - pa);
    let (on_a, on_b) = (xf(&now, la), r.on_b);
    Some(Nearest { on_a, on_b, distance: 0.0 - dot3(r.normal, on_b - on_a), ..r })
}

impl LooseState {
    /// The object a prop instance is, made on first need.
    fn obj_of(&mut self, world: &LooseWorld, inst: u32) -> usize {
        if let Some(&i) = self.by_inst.get(&inst) {
            return i;
        }
        let (fi, ty) = &world.instances[inst as usize];
        let i = self.objs.len();
        let n = inst as usize;
        if let Some(h) = level_hints().filter(|_| !r1()) {
            // (the level's order is the file's: the city 0, then every instance that is put in; the address is
            // the heap's)
            let level = 1 + n as i64;
            let addr = h.addr.get(&level).copied().unwrap_or(0xA000_0000 + 0x100 * n as i64);
            self.objs.push(Obj { ty: *ty, inst: Some(inst), matrix: m4(&fi.matrix), entry: None, hit: false, body: None, level, addr, veh: None, sleep: Sleep::reset(), dormant: false, amb: None });
            self.by_inst.insert(inst, i);
            return i;
        }
        self.objs.push(Obj {
            ty: *ty,
            inst: Some(inst),
            matrix: m4(&fi.matrix),
            entry: None,
            hit: false,
            body: None,
            level: rank('l', i).unwrap_or(if hint("b", n) { 1 + n as i64 } else { -1_000_000_000 + n as i64 }),
            addr: rank('a', i).unwrap_or(if hint("after", n) { 1 + n as i64 } else { -1_000_000_000 + n as i64 }),
            veh: None,
            sleep: Sleep::reset(),
            dormant: false,
            amb: None,
        });
        self.by_inst.insert(inst, i);
        i
    }

    /// Is part `c` of an object there to be met?
    fn has_part(&self, world: &LooseWorld, o: usize, c: usize) -> bool {
        let obj = &self.objs[o];
        if obj.veh.is_some() {
            return c == 0;
        }
        part_present(&world.types[obj.ty], obj.entry.map_or(0, |e| e.absent), c)
    }

    /// The centre of gravity of an object's whole bound (bound +80).
    fn cg_of(&self, world: &LooseWorld, o: usize) -> Vec3 {
        let obj = &self.objs[o];
        if let Some(vh) = obj.veh {
            return vh.cg;
        }
        obj.entry.map_or(v(world.types[obj.ty].cg), |e| e.cg)
    }

    /// An object's whole bound (0x8259ADA8 / 0x82599FB0 for an entry that has lost groups: the union of its
    /// parts' boxes through their matrices, centroid = the box's middle, radius = its half diagonal; else the
    /// file's): box min, max, centroid, radius.
    fn whole_bound(&self, world: &LooseWorld, o: usize) -> (Vec3, Vec3, Vec3, f32) {
        let obj = &self.objs[o];
        if let Some(vh) = obj.veh {
            let (mut lo, mut hi) = (vh.shape.min - Vec3::splat(0.041), vh.shape.max + Vec3::splat(0.041));
            if let Some(h) = vh.shape2 {
                lo = lo.min(h.min - Vec3::splat(0.041));
                hi = hi.max(h.max + Vec3::splat(0.041));
            }
            let c = (hi + lo) * 0.5;
            return (lo, hi, c, (hi - c).length());
        }
        let t = &world.types[obj.ty];
        let absent = obj.entry.map_or(0, |e| e.absent);
        if absent == 0 {
            return (v(t.min), v(t.max), v(t.centroid), t.radius);
        }
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(-f32::MAX));
        let mut any = false;
        for (c, part) in t.parts.iter().enumerate().rev() {
            let Some(b) = part.bound.as_ref().filter(|_| part_present(t, absent, c)) else { continue };
            let m = m4(&part.matrix);
            let mid = (v(b.min) + v(b.max)) * 0.5;
            let half = v(b.max) - mid;
            let centre = xf(&m, mid);
            let ext = Vec3::new(dot3(Vec3::new(m[0].x.abs(), m[1].x.abs(), m[2].x.abs()), half), dot3(Vec3::new(m[0].y.abs(), m[1].y.abs(), m[2].y.abs()), half), dot3(Vec3::new(m[0].z.abs(), m[1].z.abs(), m[2].z.abs()), half));
            lo = lo.min(centre - ext);
            hi = hi.max(ext + centre);
            any = true;
        }
        if !any {
            return (Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, 0.0);
        }
        let centroid = (hi + lo) * 0.5;
        let d = hi - centroid;
        (lo, hi, centroid, fma(d.z, d.z, fma(d.y, d.y, d.x * d.x)).sqrt())
    }

    /// The level's broad phase for an ACTIVE object `own` and another (0x825BF438): own's sphere, moved half
    /// the tick's way back and grown by half its length, against the other's as it stands; then the two whole
    /// boxes - each grown to hold itself at its matrix before the move, if it is active - along their six face
    /// axes. Each side: box min / max, centroid, radius, matrix now, matrix before the move (None: inactive).
    fn broad(own: &(Vec3, Vec3, Vec3, f32, [Vec3; 4], Option<[Vec3; 4]>), other: &(Vec3, Vec3, Vec3, f32, [Vec3; 4], Option<[Vec3; 4]>)) -> bool {
        let (m, n) = (&own.4, &other.4);
        let mut centre = xf(m, own.2);
        let mut radius = own.3;
        if let Some(prev) = own.5.as_ref() {
            let d = m[3] - prev[3];
            centre = centre - d * 0.5;
            let l2 = dot3(d, d);
            radius = (if l2 == 0.0 { 0.0 } else { l2.sqrt() }) * 0.5 + radius;
        }
        let oc = xf(n, other.2);
        let dd = centre - oc;
        let sum = radius + other.3;
        if !(sum * sum > dot3(dd, dd)) {
            return false;
        }
        let swept = |s: &(Vec3, Vec3, Vec3, f32, [Vec3; 4], Option<[Vec3; 4]>)| {
            let (mut h, mut c) = ((s.1 - s.0) * 0.5, (s.1 + s.0) * 0.5);
            if let Some(l) = s.5.as_ref() {
                let m = &s.4;
                let r: [Vec3; 3] = std::array::from_fn(|i| inv(m, l[i]));
                let t = inv(m, l[3] - m[3]);
                let c2 = t + r[0] * c.x + r[1] * c.y + r[2] * c.z;
                let e2 = r[0].abs() * h.x + r[1].abs() * h.y + r[2].abs() * h.z;
                let (hi, lo) = ((c + h).max(c2 + e2), (c - h).min(c2 - e2));
                c = (hi + lo) * 0.5;
                h = hi - c;
            }
            (xf(&s.4, c), h)
        };
        let ((wa, ha), (wb, hb)) = (swept(own), swept(other));
        let eps = Vec3::splat(1e-6);
        let r: [Vec3; 3] = std::array::from_fn(|i| Vec3::new(dot3(m[i], n[0]), dot3(m[i], n[1]), dot3(m[i], n[2])));
        let ab: [Vec3; 3] = std::array::from_fn(|i| r[i].abs() + eps);
        let tw = wa - wb;
        let t = Vec3::new(dot3(tw, n[0]), dot3(tw, n[1]), dot3(tw, n[2]));
        let q = Vec3::ZERO - Vec3::new(dot3(r[0], t), dot3(r[1], t), dot3(r[2], t));
        let w = Vec3::new(dot3(Vec3::new(r[0].x, r[1].x, r[2].x), q), dot3(Vec3::new(r[0].y, r[1].y, r[2].y), q), dot3(Vec3::new(r[0].z, r[1].z, r[2].z), q));
        let col = |k: usize| Vec3::new(ab[0][k], ab[1][k], ab[2][k]);
        (0..3).all(|i| ha[i] + dot3(ab[i], hb) > q[i].abs()) && (0..3).all(|k| hb[k] + dot3(col(k), ha) > w[k].abs())
    }

    fn broad_side(&self, world: &LooseWorld, o: usize) -> (Vec3, Vec3, Vec3, f32, [Vec3; 4], Option<[Vec3; 4]>) {
        let (lo, hi, c, r) = self.whole_bound(world, o);
        (lo, hi, c, r, self.objs[o].matrix, self.objs[o].body.as_ref().map(|b| b.prev))
    }

    /// One side's part as the detector needs it: its core and margin, its bound's centre of
    /// gravity and box, its matrix now and as the last collide is to have had it.
    fn shape<'a>(&self, world: &'a LooseWorld, car: &CarIn<'a>, who: Who, part: u16) -> Option<Shape<'a>> {
        match who {
            Who::Car if part != 0 => {
                // (RA7: a wheel's part, the composite's parts 1..)
                let w = car.wheels.get(part as usize - 1).copied().flatten()?;
                Some(Shape { core: Core::Wheel(w.core), margin: super::gjk::WHEEL_MARGIN, cg: Vec3::ZERO, bbox: (Vec3::ZERO - w.half, w.half), now: w.now, last: w.last })
            }
            Who::Car => {
                let core = Core::Hull(HullCore { corners: car.hull.scanned(), rows: [car.now[0], car.now[1], car.now[2]], at: car.now[3] });
                Some(Shape { core, margin: car.hull_margin, cg: car.hull_cg, bbox: (car.hull.min - Vec3::splat(0.041), car.hull.max + Vec3::splat(0.041)), now: car.now, last: car.last })
            }
            Who::Obj(o) => {
                let obj = &self.objs[o];
                if let Some(vh) = obj.veh {
                    // (stage R: a vehicle's one polyhedron, 0x825E0418 as the car's hull; an ambient
                    // trailer's composite: each of its two, at the identity part matrix)
                    if part >= vh.parts() || !vh.here {
                        return None;
                    }
                    let now = obj.matrix;
                    let h = vh.part(part);
                    let core = Core::Hull(HullCore { corners: h.scanned(), rows: [now[0], now[1], now[2]], at: now[3] });
                    return Some(Shape { core, margin: f32::from_bits(0x3d23_d70a), cg: vh.cg, bbox: (h.min - Vec3::splat(0.041), h.max + Vec3::splat(0.041)), now, last: obj.body.map_or(obj.matrix, |b| b.last) });
                }
                let t = &world.types[obj.ty];
                if !part_present(t, obj.entry.map_or(0, |e| e.absent), part as usize) {
                    return None;
                }
                let p = &t.parts[part as usize];
                let b = p.bound.as_ref()?;
                if !matches!(b.kind, 0 | 1 | 3) {
                    return None;
                }
                let pm = m4(&p.matrix);
                let now = super::gjk::world_matrix(&pm, &obj.matrix);
                let last = super::gjk::world_matrix(&pm, &obj.body.map_or(obj.matrix, |b| b.last));
                let (core, margin) = part_core(b, &now);
                Some(Shape { core: Core::Part(core), margin, cg: v(b.cg), bbox: (v(b.min), v(b.max)), now, last })
            }
            Who::City => None,
        }
    }

    /// A loose body's part at the matrix the body had as the tick began (its collider's +32; `PBody::prev`).
    fn prev_part(&self, world: &LooseWorld, who: Who, part: u16) -> Option<[Vec3; 4]> {
        let Who::Obj(o) = who else { return None };
        let b = self.objs[o].body.as_ref()?;
        let p = world.types[self.objs[o].ty].parts.get(part as usize)?;
        Some(super::gjk::world_matrix(&m4(&p.matrix), &b.prev))
    }

    fn parts_of(&self, world: &LooseWorld, car: &CarIn, who: Who) -> Vec<u16> {
        match who {
            Who::Obj(o) if self.objs[o].veh.is_some() => (0..self.objs[o].veh.unwrap().parts()).collect(),
            Who::Obj(o) => (0..world.types[self.objs[o].ty].parts.len() as u16).collect(),
            Who::Car => (0..=car.wheels.len() as u16).collect(),
            _ => vec![0],
        }
    }

    /// The car's level index and its instance's address (0, 0 where no recording says).
    pub fn car_place(&self) -> (i64, i64) {
        level_hints().filter(|_| !r1()).map_or((0, 0), |h| h.car)
    }

    /// The city instance's address (below every other where no recording says: the recorded ones are).
    fn city_addr(&self) -> i64 {
        level_hints().filter(|_| !r1()).and_then(|h| h.city).unwrap_or(i64::MIN)
    }

    /// The instance a new cache entry gets (0x90 bytes each, from the top of their pool downwards).
    fn entry_instance(&mut self) -> Option<i64> {
        let n = self.entries_made;
        self.entries_made += 1;
        level_hints().filter(|_| !r1()).and_then(|h| h.pool).map(|top| top - 0x90 * n)
    }

    /// The level index a new object gets: the next of the level's free index list (0x82573200).
    fn new_index(&mut self) -> Option<i64> {
        let h = level_hints().filter(|_| !r1())?;
        let k = REPLAY_K.load(std::sync::atomic::Ordering::Relaxed);
        if self.indices_taken.0 != k {
            self.indices_taken = (k, 0);
        }
        let n = self.indices_taken.1;
        self.indices_taken.1 += 1;
        h.free.get(&k).and_then(|f| f.get(n)).copied()
    }

    fn level(&self, who: Who) -> i64 {
        match who {
            Who::Car => self.car_place().0,
            Who::Obj(o) => self.objs[o].level,
            Who::City => i64::MIN,
        }
    }

    /// The contacts of two objects' parts (0x825CCE00 with 0x825CC930 per pair of parts): B's
    /// parts outside, A's inside; a pair of parts whose boxes meet, with no contact yet and one
    /// of them a prop that has not broken, goes through the swept test, every other through the
    /// detector.
    fn pair(&mut self, world: &LooseWorld, car: &CarIn, a: Who, b: Who) {
        // (record byte +280: an mcPropInst that stands loose has flag 2; a piece's own instance none)
        let is_prop = |w: Who| matches!(w, Who::Obj(o) if self.objs[o].inst.is_some());
        let sweeps = (is_prop(a) || is_prop(b)) && !(hint("x", 6) && a != Who::Car && b != Who::Car);
        for pb in self.parts_of(world, car, b) {
            let Some(sb) = self.shape(world, car, b, pb) else { continue };
            for pa in self.parts_of(world, car, a) {
                // (stage R: only the car's hull is taken against a vehicle - no wheel contact with one is kept
                // in the recordings)
                let veh_pair = [a, b].into_iter().any(|w| matches!(w, Who::Obj(o) if self.objs[o].veh.is_some()));
                if veh_pair && ((a == Who::Car && pa != 0) || (b == Who::Car && pb != 0)) {
                    continue;
                }
                let Some(sa) = self.shape(world, car, a, pa) else { continue };
                let at = self.manis.iter().position(|m| m.a == a && m.b == b && m.part_a == pa && m.part_b == pb);
                if !boxes_meet(&sa, &sb) && !hint("x", 7) {
                    continue;
                }
                if let Some(k) = at {
                    self.manis[k].visited = true;
                }
                let count = at.map_or(0, |k| self.manis[k].slots.len());
                // (the instances' last matrices, 0x82589948: the collider's +96 only while the other instance has no
                // collider record - two bodies in the simulator are each taken at +32, the matrix they had as the tick
                // began: V s_rtv s390 / s391, 11286 x 22286 / 22320)
                let both = [a, b].into_iter().all(|w| matches!(w, Who::Obj(o) if self.objs[o].body.is_some() && self.objs[o].veh.is_none()));
                // (and an active side at +32 when the other is inactive with level byte bit 8 - manifold +1249 / +1250,
                // an ambient car with inst +140 bit 0x80 moved this frame)
                let posed = |w: Who| matches!(w, Who::Obj(o) if self.objs[o].body.is_none() && self.objs[o].amb.as_ref().is_some_and(|x| x.posed));
                let prop_body = |w: Who| matches!(w, Who::Obj(o) if self.objs[o].body.is_some() && self.objs[o].veh.is_none());
                let (la, lb) = if both {
                    (self.prev_part(world, a, pa).unwrap_or(sa.last), self.prev_part(world, b, pb).unwrap_or(sb.last))
                } else {
                    let la = if posed(b) && prop_body(a) { self.prev_part(world, a, pa).unwrap_or(sa.last) } else { sa.last };
                    let lb = if posed(a) && prop_body(b) { self.prev_part(world, b, pb).unwrap_or(sb.last) } else { sb.last };
                    (la, lb)
                };
                let found: Option<(Vec3, Vec3, Vec3, f32, Nearest)> = if count == 0 && sweeps {
                    sweep(&sa.core, sa.margin, sa.cg, &sa.now, &la, &sb.core, sb.margin, sb.cg, &sb.now, &lb).filter(|s| !(s.lambda >= 1.0)).map(|s| {
                        let (qa, qb) = (xf(&sa.now, s.local_a), xf(&sb.now, s.local_b));
                        (s.normal, qa, qb, 0.0 - dot3(s.normal, qb - qa), s.last)
                    })
                } else {
                    let mut dir = Vec3::Z;
                    convex_contact(&sa.core, sa.margin, &sb.core, sb.margin, &mut dir, true).map(|r| (r.normal, r.on_a, r.on_b, r.distance, r))
                };
                if dbg() {
                    eprintln!("LOOSEPAIR tick {} {:?}/{pa} x {:?}/{pb} swept {} -> {:?}", self.tick, a, b, count == 0 && sweeps, found.map(|f| (f.0, f.3)));
                }
                let Some((normal, qa, qb, dist, r)) = found else { continue };
                // (the sink, 0x825C65E8: nothing farther than 2 cm)
                if !(f32::from_bits(0x3ca3_d70a) > dist) {
                    continue;
                }
                let (local_a, local_b) = (inv(&sa.now, qa - sa.now[3]), inv(&sb.now, qb - sb.now[3]));
                if dbg() {
                    eprintln!("LOOSECON tick {} {:?}/{pa} x {:?}/{pb} depth {:?} la {:?} lb {:?} pa {:?} pb {:?} n {:?}", self.tick, a, b, 0.0 - dist, local_a, local_b, qa, qb, normal);
                }
                // (which of the hull's triangles: by the corners the loop ended on and the
                // contact's direction in the hull's frame - away from the other part)
                let element = if (b == Who::Car && pb != 0) || (a == Who::Car && pa != 0) {
                    0
                } else if b == Who::Car {
                    car.hull.element(&r.corners_b.0[..r.corners_b.1], inv(&car.now, normal))
                } else if a == Who::Car {
                    car.hull.element(&r.corners.0[..r.corners.1], inv(&car.now, Vec3::ZERO - normal))
                } else {
                    0
                };
                if dbg() && ((a == Who::Car && pa == 0) || (b == Who::Car && pb == 0)) {
                    eprintln!("LOOSEELEM tick {} element {element} deep {} corners {:?} / {:?}", self.tick, r.deep, &r.corners.0[..r.corners.1], &r.corners_b.0[..r.corners_b.1]);
                }
                // (stage R: and which of a vehicle's - side B: the direction +n in its frame)
                let veh_surface = match (a, b) {
                    (_, Who::Obj(o)) if self.objs[o].veh.is_some() => Some(self.objs[o].veh.unwrap().part(pb)).map(|h| h.surface_of(h.element(&r.corners_b.0[..r.corners_b.1], inv(&sb.now, normal)))),
                    (Who::Obj(o), _) if self.objs[o].veh.is_some() && self.objs[o].amb.is_none() => Some(self.objs[o].veh.unwrap().part(pa)).map(|h| h.surface_of(h.element(&r.corners.0[..r.corners.1], inv(&sa.now, Vec3::ZERO - normal)))),
                    _ => None,
                };
                // (an ambient car as A: its own triangle's material, kept apart from B's)
                let amb_surface = match a {
                    Who::Obj(o) if self.objs[o].amb.is_some() => Some(self.objs[o].veh.unwrap().part(pa)).map(|h| h.surface_of(h.element(&r.corners.0[..r.corners.1], inv(&sa.now, Vec3::ZERO - normal)))),
                    _ => None,
                };
                let side = |local: Vec3, point: Vec3| Side { body: None, point, local, rows: [Vec3::X, Vec3::Y, Vec3::Z], margin: 0.04, core: Vec3::ZERO };
                let fresh = Touch { depth: 0.0 - dist, a: side(local_a, qa), b: side(local_b, qb), normal, back: Vec3::ZERO - normal, ..Default::default() };
                let k = at.unwrap_or_else(|| {
                    self.manis.push(Mani { a, b, part_a: pa, part_b: pb, cell: 0, stale: false, visited: true, island: 0, order: 0, slots: Vec::new(), rope: false });
                    self.manis.len() - 1
                });
                self.manis[k].insert(fresh, element, veh_surface.unwrap_or(Surface::Prop), amb_surface.unwrap_or(Surface::Prop));
            }
        }
    }

    /// The tick's collide of everything loose (0x8258D9F0): the car's hull against the props and
    /// pieces about it, the pieces against each other and against the props they reach, and
    /// every loose body's parts against the city.
    pub fn collide(&mut self, world: &LooseWorld, car: &CarIn, city: &super::collision::WorldCollider, car_city: bool) {
        // (the manifolds of a pair whose instance broke last tick went with the pair)
        self.manis.retain(|m| !m.stale);
        // (a level shared by the cars: what is loose meets what is loose and the city once a tick, in the first
        // car's collide; the later cars' keep those pairs as they are)
        let shared_later = self.live && self.later;
        for m in self.manis.iter_mut() {
            m.visited = shared_later && m.a != Who::Car && m.b != Who::Car;
        }
        // (the constraints first, once an update: 0x82591210 ahead of the collide)
        if !shared_later {
            self.ropes_touch();
        }
        // (the level's props about the car and about every loose body)
        let mut about = vec![car.now[3]];
        // (and about an inactive ambient car with level byte bit 8: its pairs with inactive props)
        about.extend(self.objs.iter().filter(|o| o.body.is_some() || o.amb.as_ref().is_some_and(|a| a.posed)).map(|o| o.matrix[3]));
        for (k, p) in about.into_iter().enumerate() {
            for inst in world.near(p, 8.0) {
                let (fi, ty) = &world.instances[inst as usize];
                // (only what can be knocked: level state 1)
                if fi.state != 1 {
                    continue;
                }
                let t = &world.types[*ty];
                let centre = xf(&m4(&fi.matrix), v(t.centroid));
                let reach = t.radius + if k == 0 { 3.2 } else { 1.5 };
                // (stage R, agent D: MCLA_REPLAY_OTHERS_ONLY - a one-step check of vehicle ticks takes no prop in:
                // what this port's own run knocked loose before the checked ticks is not the recording's)
                if (centre - p).length_squared() <= reach * reach && !others_only() {
                    self.obj_of(world, inst);
                }
            }
        }
        // (the pairs, the lower level index A)
        let n = self.objs.len();
        let mut pairs: Vec<(Who, Who)> = Vec::new();
        let ball = |o: &Obj| {
            // (an ambient car: its bound's box)
            if let Some(vh) = o.veh {
                let (mut lo, mut hi) = (vh.shape.min, vh.shape.max);
                if let Some(h) = vh.shape2 {
                    lo = lo.min(h.min);
                    hi = hi.max(h.max);
                }
                return (xf(&o.matrix, (lo + hi) * 0.5), ((hi - lo) * 0.5).length());
            }
            let t = &world.types[o.ty];
            (xf(&o.matrix, v(t.centroid)), t.radius)
        };
        // (RA8: the pairs by the level's own test, 0x825BF438 - MCLA_DBG_RA8: by plain balls, as before)
        let exact_broad = !super::vehsim::dbg_flag("MCLA_DBG_RA8");
        let car_side = {
            let (lo, hi) = car.bound;
            let c = (hi + lo) * 0.5;
            let d = hi - c;
            (lo, hi, c, fma(d.z, d.z, fma(d.y, d.y, d.x * d.x)).sqrt(), car.now, Some(car.last))
        };
        for o in 0..n {
            if let Some(vh) = self.objs[o].veh {
                // (stage R: a vehicle about the car - the parts' boxes decide)
                if vh.here {
                    pairs.push(if self.objs[o].level < self.car_place().0 { (Who::Obj(o), Who::Car) } else { (Who::Car, Who::Obj(o)) });
                }
                continue;
            }
            let (c, r) = ball(&self.objs[o]);
            let near = if exact_broad {
                // (two active ones: the lower index asks)
                let side = self.broad_side(world, o);
                if side.5.is_some() && self.objs[o].level < self.car_place().0 { Self::broad(&side, &car_side) } else { Self::broad(&car_side, &side) }
            } else {
                (c - car.now[3]).length_squared() <= (r + 3.2) * (r + 3.2)
            };
            if near {
                pairs.push(if self.objs[o].level < self.car_place().0 { (Who::Obj(o), Who::Car) } else { (Who::Car, Who::Obj(o)) });
            }
        }
        // (a pair is made only with a body in it: walked from the bodies - the level keeps every prop a
        // car came near - and taken in the order of the two indices, as a walk over every pair would;
        // two inactive objects only when one has level byte bit 8, 0x8258D9F0: an ambient car with
        // inst +140 bit 0x80 moved this frame, `ambient_posed`)
        let posed = |o: &Obj| o.body.is_none() && o.amb.as_ref().is_some_and(|a| a.posed);
        let start = |o: &Obj| o.body.is_some() || posed(o);
        let mut cand: Vec<(usize, usize)> = Vec::new();
        for o in 0..if shared_later { 0 } else { n } {
            if !start(&self.objs[o]) || (self.objs[o].veh.is_some() && self.objs[o].amb.is_none()) {
                continue;
            }
            for q in 0..n {
                if q != o && !(q < o && start(&self.objs[q])) {
                    cand.push((o.min(q), o.max(q)));
                }
            }
        }
        cand.sort_unstable();
        for (o1, o2) in cand {
            {
                let (a, b) = (&self.objs[o1], &self.objs[o2]);
                // (stage R: vehicle x prop, vehicle x vehicle: not ported - but an ambient car's, with what is
                // loose and with another ambient car, active or not)
                if (a.veh.is_some() && a.amb.is_none()) || (b.veh.is_some() && b.amb.is_none()) {
                    continue;
                }
                if a.body.is_none() && b.body.is_none() && !posed(a) && !posed(b) {
                    continue;
                }
                let ((ca, ra), (cb, rb)) = (ball(a), ball(b));
                if exact_broad {
                    let (sa, sb) = (self.broad_side(world, o1), self.broad_side(world, o2));
                    let found = match (sa.5.is_some(), sb.5.is_some()) {
                        (true, true) => if a.level < b.level { Self::broad(&sa, &sb) } else { Self::broad(&sb, &sa) },
                        (true, false) => Self::broad(&sa, &sb),
                        _ => Self::broad(&sb, &sa),
                    };
                    if !found {
                        continue;
                    }
                } else if (ca - cb).length_squared() > (ra + rb + 0.3) * (ra + rb + 0.3) {
                    continue;
                }
                pairs.push(if a.level < b.level { (Who::Obj(o1), Who::Obj(o2)) } else { (Who::Obj(o2), Who::Obj(o1)) });
            }
        }
        for (a, b) in pairs {
            self.pair(world, car, a, b);
        }
        // (every loose body's parts against the city)
        for o in 0..if shared_later { 0 } else { n } {
            // (stage R: a vehicle's own contacts with the city are not made - an ambient car's are)
            if self.objs[o].body.is_none() || (self.objs[o].veh.is_some() && self.objs[o].amb.is_none()) {
                continue;
            }
            for c in self.parts_of(world, car, Who::Obj(o)) {
                let Some(s) = self.shape(world, car, Who::Obj(o), c) else { continue };
                let mut hits: Vec<super::hull::PartHit> = Vec::new();
                if dbg() && self.objs[o].veh.is_none() {
                    let t = &world.types[self.objs[o].ty];
                    let p = &t.parts[c as usize];
                    if let Some(bd) = p.bound.as_ref() {
                        eprintln!("LOOSESHAPE tick {} obj {o} part {c} kind {} round {:e} len {:e} margin {:e} centroid {:?} min {:?} max {:?} pm {:?} now {:?} last {:?}", self.tick, bd.kind, bd.round, bd.length, bd.margin, bd.centroid, bd.min, bd.max, p.matrix, s.now, s.last);
                    }
                }
                // (pair byte +280: a loose prop's every polygon goes through the swept test - V s_rtv 11286, s386 / s389 /
                // s391: the city's points lie where the part was first within 2 cm, 4 to 7 cm along the ground)
                let sweeps = self.objs[o].inst.is_some();
                let swept = |local: &[Vec3; 3], face: Vec3, bisectors: [Vec3; 3], origin: Vec3| sweep_polygon(&s.core, s.margin, s.cg, &s.now, &s.last, local, face, bisectors, origin);
                super::hull::part_touches(&s.core, s.margin, s.bbox, &s.now, &s.last, city, if sweeps { Some(&swept) } else { None }, |h| hits.push(h));
                for m in self.manis.iter_mut() {
                    if m.a == Who::Obj(o) && m.b == Who::City && m.part_a == c {
                        m.visited = true;
                    }
                }
                let own = self.objs[o].veh.filter(|_| self.objs[o].amb.is_some()).map(|vh| vh.part(c));
                for h in hits {
                    let surface_a = own.map_or(Surface::Prop, |sh| sh.surface_of(sh.element(&h.corners.0[..h.corners.1], inv(&s.now, Vec3::ZERO - h.normal))));
                    let a = Side { body: None, point: h.on_a, local: h.local, rows: [Vec3::X, Vec3::Y, Vec3::Z], margin: 0.04, core: Vec3::ZERO };
                    let b = Side { body: None, point: h.on_b, local: h.on_b, rows: [Vec3::X, Vec3::Y, Vec3::Z], margin: 0.04, core: Vec3::ZERO };
                    let fresh = Touch { depth: h.depth, a, b, normal: h.normal, back: Vec3::ZERO - h.normal, ..Default::default() };
                    let k = self.manis.iter().position(|m| m.a == Who::Obj(o) && m.b == Who::City && m.part_a == c && m.cell == h.cell).unwrap_or_else(|| {
                        self.manis.push(Mani { a: Who::Obj(o), b: Who::City, part_a: c, part_b: 0, cell: h.cell, stale: false, visited: true, island: 0, order: 0, slots: Vec::new(), rope: false });
                        self.manis.len() - 1
                    });
                    if dbg() {
                        eprintln!("LOOSECITY tick {} obj {o} part {c} cell {} polygon {} depth {:?} n {:?} local {:?} on_a {:?} on_b {:?}", self.tick, h.cell, h.polygon, h.depth, h.normal, h.local, h.on_a, h.on_b);
                    }
                    self.manis[k].insert(fresh, 0, h.surface, surface_a);
                }
            }
        }
        self.manis.retain(|m| m.visited && !m.slots.is_empty());
        // (an inactive ambient car in a manifold with contacts is activated - no threshold, before the
        // islands and the solver: 0x8258D9F0 at 0x8258E07C -> 0x8258D3F8)
        let woken: Vec<usize> = self.manis.iter().filter(|m| !m.rope).flat_map(|m| [m.a, m.b]).filter_map(|w| if let Who::Obj(o) = w { Some(o) } else { None }).filter(|&o| self.objs[o].amb.is_some() && self.objs[o].body.is_none()).collect();
        for o in woken {
            self.ambient_activate(o);
        }
        // (the simulator's list emptied after the collide: level byte bit 8 cleared, sub_8258E4D0 -> sub_82588220)
        if !shared_later {
            for o in self.objs.iter_mut() {
                if let Some(a) = o.amb.as_mut() {
                    a.posed = false;
                }
            }
        }
        // (the islands, 0x825D80E0 at the end of the collide: every pair that holds a contact NOW joins its
        // two objects - a contact that the update's refresh then drops has still joined them. The pairs come
        // in the cache's order, (lower, higher level index); each is a node of a union-find forest whose
        // roots are the islands; FIND 0x825D7FD0 hangs every node it passes under the root, at the tail of
        // the root's children; UNION 0x825D8030 by rank. The update walks each tree in pre-order.)
        let n_obj = self.objs.len() + 1;
        let node = |w: Who| match w {
            Who::Car => Some(0usize),
            Who::Obj(o) => Some(o + 1),
            Who::City => None,
        };
        let mut pairs: Vec<((i64, i64), Who, Who)> = Vec::new();
        if car_city {
            pairs.push(((i64::MIN, self.car_place().0), Who::Car, Who::City));
        }
        for m in self.manis.iter().filter(|m| !m.rope) {
            let (la, lb) = (self.level(m.a), self.level(m.b));
            let key = (la.min(lb), la.max(lb));
            if !pairs.iter().any(|p| p.0 == key) {
                pairs.push((key, m.a, m.b));
            }
        }
        pairs.sort_by_key(|p| p.0);
        // (the constraints' unions come first: 0x82591210 joins its pair before the collide)
        let mut first: Vec<((i64, i64), Who, Who)> = Vec::new();
        for m in self.manis.iter().filter(|m| m.rope) {
            let (la, lb) = (self.level(m.a), self.level(m.b));
            let key = (la.min(lb), la.max(lb));
            if !first.iter().any(|p| p.0 == key) {
                first.push((key, m.a, m.b));
            }
        }
        pairs.retain(|p| !first.iter().any(|q| q.0 == p.0));
        first.extend(pairs);
        let pairs = first;
        let np = pairs.len();
        let mut parent: Vec<Option<usize>> = vec![None; np];
        let mut kids: Vec<Vec<usize>> = vec![Vec::new(); np];
        let mut rank = vec![0u32; np];
        let mut roots: Vec<usize> = Vec::new();
        let mut link: Vec<Option<usize>> = vec![None; n_obj];
        fn rehang(parent: &mut [Option<usize>], kids: &mut [Vec<usize>], roots: &mut Vec<usize>, nd: usize, to: usize) {
            if parent[nd] == Some(to) {
                return;
            }
            match parent[nd] {
                Some(p) => kids[p].retain(|x| *x != nd),
                None => roots.retain(|x| *x != nd),
            }
            parent[nd] = Some(to);
            kids[to].push(nd);
        }
        fn find_root(parent: &mut [Option<usize>], kids: &mut [Vec<usize>], roots: &mut Vec<usize>, x: usize) -> usize {
            if parent[x].is_none() {
                return x;
            }
            let mut r = x;
            while let Some(p) = parent[r] {
                r = p;
            }
            let mut nd = x;
            while nd != r {
                let next = parent[nd].unwrap();
                rehang(parent, kids, roots, nd, r);
                nd = next;
            }
            r
        }
        fn union(parent: &mut [Option<usize>], kids: &mut [Vec<usize>], roots: &mut Vec<usize>, rank: &mut [u32], x: usize, y: usize) -> usize {
            if x == y {
                return x;
            }
            if rank[x] > rank[y] {
                rehang(parent, kids, roots, y, x);
                x
            } else if rank[x] < rank[y] {
                rehang(parent, kids, roots, x, y);
                y
            } else {
                rehang(parent, kids, roots, y, x);
                rank[x] += 1;
                x
            }
        }
        for k in 0..np {
            roots.push(k);
            let mut r = find_root(&mut parent, &mut kids, &mut roots, k);
            let (a, b) = (node(pairs[k].1), node(pairs[k].2));
            if let Some(a) = a {
                if let Some(l) = link[a] {
                    let x = find_root(&mut parent, &mut kids, &mut roots, l);
                    r = union(&mut parent, &mut kids, &mut roots, &mut rank, x, r);
                }
                link[a] = Some(r);
            }
            if let Some(b) = b.filter(|b| Some(*b) != a) {
                if let Some(l) = link[b] {
                    let x = find_root(&mut parent, &mut kids, &mut roots, l);
                    r = union(&mut parent, &mut kids, &mut roots, &mut rank, x, r);
                }
                link[b] = Some(r);
                if let Some(a) = a {
                    link[a] = Some(r);
                }
            }
        }
        // (pre-order; the island of the car is number 0, the others by their roots)
        let mut place = vec![(0usize, 0usize); np];
        let mut count = 0;
        for &r in roots.iter() {
            let mut stack = vec![r];
            while let Some(nd) = stack.pop() {
                place[nd] = (r + 1, count);
                count += 1;
                for c in kids[nd].iter().rev() {
                    stack.push(*c);
                }
            }
        }
        let car_root = pairs.iter().position(|p| p.1 == Who::Car || p.2 == Who::Car).map(|k| place[k].0);
        self.car_city_order = pairs.iter().position(|p| p.1 == Who::Car && p.2 == Who::City).map(|k| place[k].1);
        for mi in 0..self.manis.len() {
            let (la, lb) = (self.level(self.manis[mi].a), self.level(self.manis[mi].b));
            let key = (la.min(lb), la.max(lb));
            let k = pairs.iter().position(|p| p.0 == key).unwrap();
            let m = &mut self.manis[mi];
            m.island = if Some(place[k].0) == car_root { 0 } else { place[k].0 };
            m.order = place[k].1;
        }
    }

    /// The matrix of one side's part as the tick's update has it (0x825A38E0).
    fn part_matrix(&self, world: &LooseWorld, who: Who, part: u16, car: &CarIn) -> [Vec3; 4] {
        match who {
            Who::Car if part != 0 => car.wheels.get(part as usize - 1).copied().flatten().map_or(car.now, |w| w.now),
            Who::Car => car.now,
            Who::Obj(o) => {
                let obj = &self.objs[o];
                if obj.veh.is_some() {
                    return obj.matrix;
                }
                super::gjk::world_matrix(&m4(&world.types[obj.ty].parts[part as usize].matrix), &obj.matrix)
            }
            Who::City => [Vec3::X, Vec3::Y, Vec3::Z, Vec3::ZERO],
        }
    }

    /// The kept contacts brought up to the two matrices as they stand (0x825A3070 with
    /// 0x825A28C8), their materials (0x825A2C88) and the instances' callbacks: a prop's sets the
    /// elasticity of its type on all its contacts (0x82735A90); the car's lays a normal flat
    /// that would push it down against a prop that stands (0x82332D30) - and, fast enough,
    /// feels nothing of a prop that broke two seconds ago and more (0x82332AC8).
    pub fn refresh(&mut self, world: &LooseWorld, car: &CarIn) {
        for k in 0..self.manis.len() {
            let (ma, mb) = (self.part_matrix(world, self.manis[k].a, self.manis[k].part_a, car), self.part_matrix(world, self.manis[k].b, self.manis[k].part_b, car));
            let m = &mut self.manis[k];
            let city = m.b == Who::City;
            if m.rope {
                // (0x825A28C8 on a contact of age 0: its points and depth left; +144 zeroed, age 1)
                for s in m.slots.iter_mut() {
                    s.t.target = Vec3::ZERO;
                    s.age = 1;
                }
                continue;
            }
            for s in m.slots.iter_mut().rev() {
                if s.age != 0 {
                    s.t.a.point = xf(&ma, s.t.a.local);
                    if city {
                        s.t.b.point = s.t.b.local;
                    } else {
                        s.t.b.point = xf(&mb, s.t.b.local);
                    }
                    s.t.depth = dot3(s.t.a.point - s.t.b.point, s.t.back);
                }
                s.t.target = Vec3::ZERO;
                s.age = s.age.saturating_add(1);
            }
            let mut worst = (0.0f32, None);
            let mut i = 0;
            while i < m.slots.len() {
                let s = m.slots[i];
                if !(s.t.depth > f32::from_bits(0xbd23_d70a)) {
                    m.slots.swap_remove(i);
                    continue;
                }
                if s.age > 1 {
                    let q = -(s.t.back * s.t.depth - s.t.a.point);
                    let slid = s.t.b.point - q;
                    let d2 = dot3(slid, slid);
                    if !(worst.0 >= d2) && !(f32::from_bits(0x39d1_b717) >= d2) {
                        worst = (d2, Some(i));
                    }
                }
                i += 1;
            }
            if let Some(i) = worst.1 {
                m.slots.swap_remove(i);
            }
        }
        self.manis.retain(|m| !m.slots.is_empty());
        let prop = surface_touch(Surface::Prop);
        let partners = |a: Who, b: Who, objs: &Vec<Obj>| match (a, b) {
            (Who::Obj(x), Who::Obj(y)) => match (objs[x].amb.as_ref(), objs[y].amb.as_ref()) {
                (Some(p), Some(q)) => p.partner == Some(q.uid) || q.partner == Some(p.uid),
                _ => false,
            },
            _ => false,
        };
        for m in self.manis.iter_mut() {
            // (a constraint's contact: no material, 0x825A2C88 for kind != 0 zeroes +112 only)
            if m.rope {
                for s in m.slots.iter_mut() {
                    s.t.impulse = Vec3::ZERO;
                    s.enabled = true;
                }
                continue;
            }
            // (a semi's cab and its trailer: their contacts are off every solve - slot 23, sub_826FAC50)
            let off = partners(m.a, m.b, &self.objs);
            let (who_a, who_b) = (m.a, m.b);
            let car_is_a = who_a == Who::Car;
            let with_car = car_is_a || who_b == Who::Car;
            // (a wheel's contact with something loose: the wheel's callback 0x8255D028 takes it from the solver;
            // it stays in the manifold)
            let wheel = (car_is_a && m.part_a != 0) || (who_b == Who::Car && m.part_b != 0);
            // (an mcPropInst on either side: its callback writes its type's elasticity)
            let props: Vec<usize> = [who_a, who_b].into_iter().filter_map(|w| if let Who::Obj(o) = w { Some(o) } else { None }).filter(|o| self.objs[*o].inst.is_some()).collect();
            // (stage R: a vehicle on the other side)
            let other_veh = [who_a, who_b].into_iter().find_map(|w| if let Who::Obj(o) = w { self.objs[o].veh.map(|vh| (o, vh.class)) } else { None });
            for s in m.slots.iter_mut() {
                let of = |w: Who| match w {
                    Who::Car => surface_touch(car.hull.surface_of(s.element)),
                    Who::Obj(o) if self.objs[o].amb.is_some() && w == who_a => surface_touch(s.surface_a),
                    Who::Obj(o) if self.objs[o].veh.is_some() => surface_touch(s.surface),
                    Who::Obj(_) => prop,
                    Who::City => surface_touch(s.surface),
                };
                let (ta, tb) = (of(who_a), of(who_b));
                let f = ta.0 * tb.0;
                s.t.friction = if f >= 0.0 { f } else { 0.0 };
                let e = ta.1 * tb.1;
                s.t.elasticity = if e - 1.0 >= 0.0 { 1.0 } else { e };
                s.t.impulse = Vec3::ZERO;
                s.enabled = true;
                if wheel || off {
                    s.enabled = false;
                    continue;
                }
                for o in &props {
                    s.t.elasticity = world.types[self.objs[*o].ty].elasticity;
                }
                if !with_car {
                    continue;
                }
                if let Some((o, class)) = other_veh {
                    car_meets_vehicle(car, &self.objs[o].matrix, class, car_is_a, &mut s.t);
                    continue;
                }
                let Some(&o) = props.first() else { continue };
                let obj = &self.objs[o];
                let t = &world.types[obj.ty];
                // (the car's, 0x82332AC8: a hull contact with a prop of a type that only breaks,
                // broken more than two seconds ago, is not felt above 20 m/s)
                if car.speed > 20.0 && t.min_move < 0.0 && obj.hit && obj.entry.map_or(false, |e| e.age > 2.0) {
                    s.enabled = false;
                    continue;
                }
                // (0x82332D30, against an mcPropInst: the normal as it pushes the car points down)
                let mut n = if car_is_a { s.t.normal } else { Vec3::ZERO - s.t.normal };
                if n.y < 0.0 {
                    let k = 1.0 - (obj.matrix[1].y * car.now[1].y).abs();
                    if k < 1.0 {
                        let mut y2 = n.y * n.y;
                        let rest = 1.0 - y2;
                        if rest > 0.0 {
                            n.y = k * n.y;
                            y2 = n.y * n.y;
                            let q = ((1.0 - y2) / rest).sqrt();
                            n.x *= q;
                            n.z *= q;
                        }
                        let l2 = fma(n.z, n.z, n.x * n.x) + y2;
                        if l2 > f32::from_bits(0x3f67_0a3d) && l2 < f32::from_bits(0x3f8d_1eb7) {
                            let d = dot3(n, n);
                            let e = 1.0 / d.sqrt();
                            let e = if e == f32::INFINITY { 0.0 } else { e };
                            let unit = n * ((1.0 - d * (e * e)) * (0.5 * e) + e);
                            let back = Vec3::ZERO - unit;
                            if car_is_a {
                                s.t.normal = unit;
                                s.t.back = back;
                            } else {
                                s.t.back = unit;
                                s.t.normal = back;
                            }
                        }
                    }
                }
            }
        }
        self.pieces_touched(car);
    }

}

// ---------------------------------------------------------------------------------------------
// The breaking pass (0x82593680, 0x82592D08, 0x825920F8; p1/break_decode.md).

/// 0x8248C710: how much of the impulses on its parts (`comp`, by part) breaks or moves an
/// instance: the share `t` and the group that leaves (255: the whole of it).
fn break_strength(t: &FragType, obj: &Obj, comp: &[Vec3]) -> Option<(f32, u8)> {
    let entry = obj.entry;
    let present = |g: usize| entry.map_or(true, |e| e.absent & (1 << g) == 0);
    if entry.is_none() && t.flags & 1 == 0 {
        // (a prop that goes as a whole)
        let f = t.min_move;
        if f < 0.0 {
            return None;
        }
        let mut tot = 0.0f32;
        for c in comp.iter().take(t.parts.len()) {
            tot = fma(c.y, c.y, fma(c.x, c.x, c.z * c.z)).sqrt() + tot;
        }
        if tot < f {
            return None;
        }
        return Some((f / tot, 255));
    }
    let mut gi = vec![0.0f32; t.groups.len()];
    let mut total = 0.0f32;
    fn spread(t: &FragType, present: &dyn Fn(usize) -> bool, gi: &mut [f32], mut m: f32, mut g: usize, mut dir: i32) {
        loop {
            let grp = &t.groups[g];
            if dir < 0 {
                m = grp.down * m;
            }
            if present(g) {
                gi[g] = gi[g] + m;
            }
            if dir <= 0 {
                for k in 0..grp.groups as usize {
                    let c = grp.first_group as usize + k;
                    if present(c) {
                        spread(t, present, gi, m, c, dir - 1);
                    }
                }
                if dir < 0 {
                    return;
                }
            }
            let p = grp.parent as usize;
            if grp.parent == 255 || !present(p) {
                return;
            }
            m = grp.up * m;
            dir += 1;
            g = p;
        }
    }
    for (p, c) in comp.iter().enumerate().take(t.parts.len()) {
        if c.x == 0.0 && c.y == 0.0 && c.z == 0.0 {
            continue;
        }
        let m = fma(c.y, c.y, fma(c.z, c.z, c.x * c.x)).sqrt();
        let g = group_of(t, p);
        if entry.is_some() && !present(g) {
            continue;
        }
        total = m + total;
        spread(t, &present, &mut gi, m, g, 0);
    }
    if !obj.hit && t.min_move >= 0.0 {
        let x = t.groups[0].up * gi[0];
        if x >= t.min_move {
            return Some((t.min_move / x, 255));
        }
    }
    let (mut best, mut win) = (0.0f32, 255u8);
    for g in 0..t.groups.len() {
        let x = gi[g];
        if x <= 0.0 {
            continue;
        }
        let grp = &t.groups[g];
        let s = grp.strength;
        if s < 0.0 && grp.latch == 0.0 {
            continue;
        }
        let ok = if grp.parent != 255 { entry.is_none() || present(grp.parent as usize) } else { t.root_groups > 1 && (0..t.root_groups as usize).filter(|r| present(*r)).count() >= 2 };
        if !ok {
            continue;
        }
        let ratio = if s > f32::from_bits(0x3586_37bd) { x / s } else if s < 0.0 { 0.0 } else { f32::MAX };
        if ratio > 1.0 && ratio > best {
            win = g as u8;
            best = ratio;
        }
    }
    if win == 255 {
        return None;
    }
    Some((t.groups[win as usize].strength / total, win))
}

/// The original's sort of an island's contact list (STLport std::sort, 0x82593F08: the introsort loop
/// 0x82593CC8 with the partition 0x825934D8 while a range holds more than 16, then insertion sorts
/// 0x82593928 / 0x825917B8): ascending by `key`, stable up to 16 elements, above that as its steps fall.
pub fn stl_sort<T: Clone>(a: &mut Vec<T>, key: impl Fn(&T) -> i64) {
    fn insertion<T: Clone>(a: &mut [T], lo: usize, hi: usize, key: &impl Fn(&T) -> i64) {
        for i in lo + 1..hi {
            let v = a[i].clone();
            if key(&v) < key(&a[lo]) {
                for j in (lo..i).rev() {
                    a[j + 1] = a[j].clone();
                }
                a[lo] = v;
            } else {
                let mut j = i;
                while key(&v) < key(&a[j - 1]) {
                    a[j] = a[j - 1].clone();
                    j -= 1;
                }
                a[j] = v;
            }
        }
    }
    fn quick<T: Clone>(a: &mut [T], first: usize, mut last: usize, key: &impl Fn(&T) -> i64) {
        while last - first > 16 {
            let n = last - first;
            let (ka, kb, kc) = (key(&a[first]), key(&a[first + n / 2]), key(&a[last - 1]));
            let m = if ka < kb {
                if kb < kc { first + n / 2 } else if ka < kc { last - 1 } else { first }
            } else if ka < kc {
                first
            } else if kb < kc {
                last - 1
            } else {
                first + n / 2
            };
            let kp = key(&a[m]);
            let (mut f, mut l) = (first, last);
            loop {
                while key(&a[f]) < kp {
                    f += 1;
                }
                l -= 1;
                while kp < key(&a[l]) {
                    l -= 1;
                }
                if !(f < l) {
                    break;
                }
                a.swap(f, l);
                f += 1;
            }
            quick(a, f, last, key);
            last = f;
        }
    }
    let n = a.len();
    if n > 16 {
        quick(a, 0, n, &key);
        insertion(a, 0, 16, &key);
        // (the rest without the guard: the least stands in the first sixteen)
        for i in 16..n {
            let v = a[i].clone();
            let mut j = i;
            while j > 0 && key(&v) < key(&a[j - 1]) {
                a[j] = a[j - 1].clone();
                j -= 1;
            }
            a[j] = v;
        }
    } else {
        insertion(a, 0, n, &key);
    }
}

/// The bone whose matrix a break event of group `g` follows: that of the part that stays, the first child of the
/// parent group, if that has a bone (`prop_hit_fx_rule.md` 2c; none: the instance's matrix).
fn fx_part(t: &FragType, g: usize) -> Option<u16> {
    let pg = t.groups.get(g)?.parent as usize;
    if pg == 255 {
        return None;
    }
    let c = t.groups.get(pg)?.first_child as usize;
    t.children.get(c).filter(|ch| ch.bone != -1).map(|ch| ch.bone as u16)
}

/// One contact of an island as the breaking pass sees it.
#[derive(Clone, Copy, Debug)]
pub struct Link {
    pub a: Option<Who>,
    pub b: Option<Who>,
    pub part_a: u16,
    pub part_b: u16,
    /// where it is kept: a manifold of this state and its slot, or (None) the car's own
    pub from: Option<(usize, usize)>,
    /// a contact of the car's own (`from` None): its number among the car's contacts with the city as
    /// they were handed in - the sort can move it among the car's other rows, and what the solve leaves
    /// is the contact's own
    pub car_row: usize,
}

/// Where a row of a car's island comes from.
#[derive(Clone, Copy)]
enum RowSrc {
    /// the car's contact with the city of this number
    City(usize),
    /// a slot of this manifold
    Mani(usize),
}

/// What a car's island came to.
pub struct IslandOut {
    pub touches: Vec<Touch>,
    pub links: Vec<Link>,
    pub bodies: Vec<Body>,
    /// which object each body after the car's (0) is
    pub owners: Vec<usize>,
    pub solved: solver::IslandSolved,
    /// what the car's +368 / +384 gain
    pub impulse: Vec3,
    pub turning: Vec3,
    /// the rows (numbers into `links`) in the island's second list, sorted by instance B's address
    /// (0x82594068's list for the B side, island +296)
    pub by_b: Vec<usize>,
    /// the car's runs of the callbacks (its A run, then its B run, those it has): the first row's other object
    /// active, a row on a part of the car that is not a wheel, a row with an impulse
    pub car_runs: Vec<(bool, bool, bool)>,
    /// the car's normals summed over the rows the callbacks disabled, of the car's A run and of its B run
    /// (mcCarDamage's scrape normal takes every row of its run, 0x82343F8C)
    pub off_a: Vec3,
    pub off_b: Vec3,
}

impl LooseState {
    /// Is an instance one the breaking pass lists (instance vtable[8], 0x8248AD40)?
    fn breakable(&self, world: &LooseWorld, o: usize) -> bool {
        let obj = &self.objs[o];
        if obj.veh.is_some() {
            return false;
        }
        if !obj.hit {
            return true;
        }
        let Some(e) = obj.entry else { return false };
        (0..world.types[obj.ty].groups.len()).filter(|g| e.absent & (1 << g) == 0).count() >= 2
    }

    /// The bodies of an island's contacts, the car's first, and each side's body number.
    fn bodies_of(&self, car: &Body, links: &[Link], touches: &mut [Touch]) -> (Vec<Body>, Vec<usize>) {
        let mut bodies = vec![*car];
        let mut owners: Vec<usize> = Vec::new();
        for (l, t) in links.iter().zip(touches.iter_mut()) {
            for (who, side) in [(l.a, &mut t.a), (l.b, &mut t.b)] {
                side.body = match who {
                    Some(Who::Car) => Some(0),
                    Some(Who::Obj(o)) => self.objs[o].body.map(|b| match owners.iter().position(|x| *x == o) {
                        Some(k) => k + 1,
                        None => {
                            owners.push(o);
                            bodies.push(b.as_solver());
                            owners.len()
                        }
                    }),
                    Some(Who::City) | None => None,
                };
            }
        }
        (bodies, owners)
    }

    /// The velocity solve of the car's island with what is loose in it, and the breaking pass
    /// (0x825DE200 whose apply stands back, then 0x82593680): the solve with every prop that has
    /// no body taken for immovable; the instances asked what of those impulses breaks them; the
    /// winner's share given at once to the bodies; the break; and the solve again from where the
    /// bodies then are, until nothing breaks. `city`: the car's own contacts with the world
    /// (it is A, the world has no body), which stand first or last by the addresses.
    pub fn solve_car_island(&mut self, world: &LooseWorld, car_in: &CarIn, car: &mut Body, dynm: &mut CarDyn, city: &[solver::Contact], city_rows: &[bool], city_part: &[u8], ids: &[usize], dt: f32) -> IslandOut {
        let _ = car_in;
        let (car_level, car_addr) = self.car_place();
        // (the island's contacts, 0x82594068: every contact of every manifold in the tree's pre-order - a
        // pair's sub-manifolds by (B's part, A's part), a manifold's contacts by their number - the ones the
        // callbacks disabled too; that list sorted by the address of each one's instance A with the
        // original's own sort, which is not stable above 16; the enabled ones of it are the rows.
        // `city_rows`: the car's contacts with the city in their order, false = one that is not enabled.
        // MCLA_DBG_RA5: pairs by their indices and a stable sort, as before.)
        let old_order = super::vehsim::dbg_flag("MCLA_DBG_RA5");
        let car_order = self.car_city_order.unwrap_or(0);
        // (each row: its place in the tree walk, instance A's address, instance B's, the row)
        let city_addr = self.city_addr();
        let (mut off_a, mut off_b) = (Vec3::ZERO, Vec3::ZERO);
        let mut list: Vec<((i64, i64, u64, u64), i64, i64, Option<(Link, Touch)>)> = Vec::new();
        let mut next_city = 0;
        // (where each row of the walk comes from: the car's contact with the city of that number, or a manifold)
        let mut src: Vec<RowSrc> = Vec::new();
        let city_link = |(row, c): (usize, &solver::Contact)| {
            let a = Side { body: Some(0), point: c.on_body, local: c.local, rows: car.rows, margin: 0.04, core: Vec3::ZERO };
            let b = Side { body: None, point: c.on_world, local: c.on_world, rows: [Vec3::X, Vec3::Y, Vec3::Z], margin: 0.04, core: Vec3::ZERO };
            (Link { a: Some(Who::Car), b: None, part_a: 0, part_b: 0, from: None, car_row: row }, Touch { depth: c.depth, friction: c.friction, elasticity: c.elasticity, a, b, normal: c.normal, back: Vec3::ZERO - c.normal, pushed: c.pushed, kept: c.kept, target: c.target, impulse: Vec3::ZERO, kind: 0 })
        };
        if old_order || city_rows.is_empty() {
            for (n, c) in city.iter().enumerate() {
                list.push(((if old_order { i64::MIN } else { car_order as i64 }, car_level, 0, n as u64), car_addr, city_addr, Some(city_link((n, c)))));
                src.push(RowSrc::City(n));
            }
        } else {
            for (n, on) in city_rows.iter().enumerate() {
                let row = if *on {
                    next_city += 1;
                    city.get(next_city - 1).map(|c| city_link((next_city - 1, c)))
                } else {
                    None
                };
                list.push(((car_order as i64, car_level, 0, n as u64), car_addr, city_addr, row));
                src.push(RowSrc::City(n));
            }
        }
        for &k in ids {
            let m = &self.manis[k];
            let (la, lb) = (self.level(m.a), self.level(m.b));
            let pair = if old_order { (la.min(lb), la.max(lb), if m.b == Who::City { m.cell as u64 } else { m.part_b as u64 }, m.part_a as u64) } else { (m.order as i64, 0, if m.b == Who::City { m.cell as u64 } else { m.part_b as u64 }, m.part_a as u64) };
            let key = match m.a {
                Who::Obj(o) => self.objs[o].addr,
                _ => car_addr,
            };
            let key_b = match m.b {
                Who::Obj(o) => self.objs[o].addr,
                Who::Car => car_addr,
                Who::City => city_addr,
            };
            let rows_of = |w: Who| match w {
                Who::Car => car.rows,
                Who::Obj(o) => [self.objs[o].matrix[0], self.objs[o].matrix[1], self.objs[o].matrix[2]],
                Who::City => [Vec3::X, Vec3::Y, Vec3::Z],
            };
            for (i, s) in m.slots.iter().enumerate() {
                if !s.enabled {
                    if m.a == Who::Car {
                        off_a += s.t.normal;
                    } else if m.b == Who::Car {
                        off_b += Vec3::ZERO - s.t.normal;
                    }
                    if !old_order {
                        list.push((pair, key, key_b, None));
                        src.push(RowSrc::Mani(k));
                    }
                    continue;
                }
                let mut t = s.t;
                t.a.rows = rows_of(m.a);
                t.b.rows = rows_of(m.b);
                // (the whole bound of either side is a composite or the city's grid: its margin +112)
                t.a.margin = 0.04;
                t.b.margin = 0.04;
                // (stage R: a vehicle's whole bound is its polyhedron - the thickness from a corner of it)
                // (an ambient trailer's whole bound is its composite: margin +112, no core)
                if let Who::Obj(o) = m.a {
                    if let Some(vh) = self.objs[o].veh.filter(|v| v.shape2.is_none()) {
                        t.a.margin = f32::from_bits(0x3d23_d70a);
                        t.a.core = veh_core(vh.shape, &t.a.rows, t.normal);
                    }
                }
                if let Who::Obj(o) = m.b {
                    if let Some(vh) = self.objs[o].veh.filter(|v| v.shape2.is_none()) {
                        t.b.margin = f32::from_bits(0x3d23_d70a);
                        t.b.core = veh_core(vh.shape, &t.b.rows, t.back);
                    }
                }
                // (a torn bumper's whole bound is its box: the thickness from the corner of the box less its margin
                // along the normal, 0x825E03D4's support)
                if let Who::Obj(o) = m.a {
                    if let Some(c) = piece_core(world, &self.objs[o], &t.a.rows, t.normal) {
                        t.a.core = c;
                    }
                }
                if let Who::Obj(o) = m.b {
                    if let Some(c) = piece_core(world, &self.objs[o], &t.b.rows, t.back) {
                        t.b.core = c;
                    }
                }
                list.push((pair, key, key_b, Some((Link { a: Some(m.a), b: Some(m.b), part_a: m.part_a, part_b: m.part_b, car_row: 0, from: Some((k, i)) }, t))));
                src.push(RowSrc::Mani(k));
            }
        }
        // (the rows' sources go with them)
        let mut both: Vec<_> = list.into_iter().zip(src).collect();
        both.sort_by_key(|x| x.0 .0);
        let (list, src): (Vec<_>, Vec<RowSrc>) = both.into_iter().unzip();
        if dbg() {
            eprintln!("LOOSEROWS tick {} n {} city_rows {:?} walk {:?}", self.tick, list.len(), city_rows, list.iter().map(|x| (x.0 .0, x.0 .2, x.0 .3, x.1, x.3.is_some())).collect::<Vec<_>>());
            eprintln!("LOOSEWARM tick {} {:?}", self.tick, list.iter().filter_map(|x| x.3.as_ref().map(|(l, t)| (l.a, l.b, t.depth, t.kept))).collect::<Vec<_>>());
        }
        // (the same walk sorted by instance B's address, the same sort: the list the callbacks' second
        // pass walks, 0x82590DB0 - by the rows' places in the walk)
        let mut walk_b: Vec<(i64, usize)> = list.iter().enumerate().map(|(n, x)| (x.2, n)).collect();
        let enabled: Vec<bool> = list.iter().map(|x| x.3.is_some()).collect();
        let mut tagged: Vec<(i64, usize, Option<(Link, Touch)>)> = list.into_iter().enumerate().map(|(n, x)| (x.1, n, x.3)).collect();
        if old_order {
            tagged.sort_by_key(|x| x.0);
            walk_b.sort_by_key(|x| x.0);
        } else {
            stl_sort(&mut tagged, |x| x.0);
            stl_sort(&mut walk_b, |x| x.0);
        }
        // (the car's two runs - its rows of the A-sorted list, where it is A, then of the B-sorted one - as the
        // callbacks are handed them, 0x82590DB0: every row of the run, the disabled ones too)
        let car_runs_at: [Vec<usize>; 2] = [
            tagged.iter().filter(|x| x.0 == car_addr).map(|x| x.1).collect(),
            walk_b.iter().filter(|x| x.0 == car_addr).map(|x| x.1).collect(),
        ];
        let mut row_of = vec![usize::MAX; enabled.len()];
        for (r, x) in tagged.iter().filter(|x| x.2.is_some()).enumerate() {
            row_of[x.1] = r;
        }
        let by_b: Vec<usize> = walk_b.iter().filter(|x| enabled[x.1]).map(|x| row_of[x.1]).collect();
        let list: Vec<(Link, Touch)> = tagged.into_iter().filter_map(|x| x.2).collect();
        let mut links: Vec<Link> = list.iter().map(|x| x.0).collect();
        let mut touches: Vec<Touch> = list.iter().map(|x| x.1).collect();
        let inv_dt = 1.0 / dt;
        let mut rounds_left = 64;
        // (the wake, 0x82590A60 -> 0x8258D3F8: an inactive instance that was hit before, in a contact of an island
        // with something breakable in it (island +75), gets a new body at rest. The decode has the call in the
        // breaking pass's listing (0x825914B8), after the first solve; the recording says the woken body is
        // already in the first solve - s512 / s518 of the long scrape exact only so - so it is done before the
        // rows. Whether the island +75 condition matters is not shown by the recording: the same run without it is
        // the same, bit for bit. Every manifold of the island counts, also one whose contacts are all disabled: V s_rtv
        // s473 / s496, the car's wheel 3 1.1 cm from asleep piece 22360 (contact flags 0x4d) wakes it.)
        self.wake_island(world, ids);
        loop {
            let (bodies, owners) = self.bodies_of(car, &links, &mut touches);
            let solved = solver::island_solve(&bodies, &touches, dt, inv_dt);
            solver::island_impulses(&solved, &mut touches);
            if dbg() {
                for (l, t) in links.iter().zip(touches.iter()) {
                    eprintln!("LOOSESOLVE tick {} {:?}/{} x {:?}/{} depth {:?} mu {:?} e {:?} n {:?} P {:?} J {:?} |J| {}", self.tick, l.a, l.part_a, l.b, l.part_b, t.depth, t.friction, t.elasticity, t.normal, t.kept, t.impulse, t.impulse.length());
                }
            }
            // ---- 0x825920F8: the rounds ----
            let mut j: Vec<Vec3> = touches.iter().map(|t| t.impulse).collect();
            let mut broke = false;
            for _ in 0..33 {
                // (0x825914B8: the breakable instances and the impulse on each of their parts)
                let mut listed: Vec<(usize, Vec<Vec3>)> = Vec::new();
                for (i, l) in links.iter().enumerate() {
                    for (who, part, first_minus) in [(l.a, l.part_a, false), (l.b, l.part_b, true)] {
                        let Some(Who::Obj(o)) = who else { continue };
                        if !self.breakable(world, o) {
                            continue;
                        }
                        match listed.iter().position(|x| x.0 == o) {
                            Some(k) => listed[k].1[part as usize] = listed[k].1[part as usize] + j[i],
                            None => {
                                let mut comp = vec![Vec3::ZERO; 128];
                                comp[part as usize] = if first_minus { Vec3::ZERO - j[i] } else { j[i] };
                                listed.push((o, comp));
                            }
                        }
                    }
                }
                // (0x82591708: the instance that breaks at the least share)
                let mut win: Option<(usize, f32, u8)> = None;
                let mut least = 1.0f32;
                for (o, comp) in &listed {
                    if let Some((tk, g)) = break_strength(&world.types[self.objs[*o].ty], &self.objs[*o], comp) {
                        if tk < least {
                            least = tk;
                            win = Some((*o, tk, g));
                        }
                    }
                }
                let Some((w, share, group)) = win else { break };
                if dbg() {
                    eprintln!("LOOSEBREAK tick {} obj {w} group {group} t {:?} ({:08x})", self.tick, share, share.to_bits());
                }
                // (the winner's share of every contact of it, to its body - if it has one now -
                // and, the other way, to the other's)
                let w_has_body = self.objs[w].body.is_some();
                for (i, l) in links.iter().enumerate() {
                    let s = j[i] * share;
                    for side in 0..2 {
                        let who = if side == 0 { l.a } else { l.b };
                        if who != Some(Who::Obj(w)) {
                            continue;
                        }
                        let t = &touches[i];
                        let (point, vec) = if side == 0 { (t.a.point, s) } else { (t.b.point, Vec3::ZERO - s) };
                        if w_has_body {
                            self.objs[w].body.as_mut().unwrap().impulse_now(vec, point);
                        }
                        let (other, opoint, ovec) = if side == 0 { (l.b, t.b.point, Vec3::ZERO - s) } else { (l.a, t.a.point, s) };
                        match other {
                            Some(Who::Car) => {
                                *dynm.p = *dynm.p + ovec;
                                let vel = *dynm.p * car.inv_mass;
                                *dynm.l = *dynm.l + cross(opoint - car.at, ovec);
                                let ang = ex::spin_of(*dynm.l, &dynm.rows, dynm.inverse);
                                car.vel = vel;
                                car.ang = ang;
                            }
                            Some(Who::Obj(o)) => {
                                if let Some(b) = self.objs[o].body.as_mut() {
                                    b.impulse_now(ovec, opoint);
                                }
                            }
                            Some(Who::City) | None => {}
                        }
                    }
                }
                // (the break / the activation, 0x824900A8)
                let target = self.break_apart(world, w, group);
                // (the contacts of the parts that left now name the new instance, 0x82593030)
                if let Some((new, bits)) = target {
                    let t = &world.types[self.objs[w].ty];
                    for l in links.iter_mut() {
                        for (who, part) in [(&mut l.a, l.part_a), (&mut l.b, l.part_b)] {
                            if *who == Some(Who::Obj(w)) && bits & (1 << group_of(t, part as usize)) != 0 {
                                *who = Some(Who::Obj(new));
                            }
                        }
                    }
                    for m in self.manis.iter_mut() {
                        if m.a == Who::Obj(w) && bits & (1 << group_of(t, m.part_a as usize)) != 0 {
                            m.a = Who::Obj(new);
                            m.stale = true;
                        }
                        if m.b == Who::Obj(w) && bits & (1 << group_of(t, m.part_b as usize)) != 0 {
                            m.b = Who::Obj(new);
                            m.stale = true;
                        }
                    }
                }
                let k1 = 1.0 - share;
                for (i, l) in links.iter().enumerate() {
                    if l.a == Some(Who::Obj(w)) || l.b == Some(Who::Obj(w)) {
                        j[i] = j[i] * k1;
                    }
                }
                broke = true;
            }
            rounds_left -= 1;
            if broke && rounds_left > 0 {
                continue;
            }
            // (nothing broke: the impulses go to the bodies' accumulators, 0x825A20E8)
            // (a body woken above is in the apply's list now: the owners are the solve's and the woken)
            let (bodies, owners) = self.bodies_of(car, &links, &mut touches);
            let given = solver::island_apply(&bodies, &touches);
            for (k, o) in owners.iter().enumerate() {
                let b = self.objs[*o].body.as_mut().unwrap();
                b.jf = b.jf + given[k + 1].0;
                b.jt = b.jt + given[k + 1].1;
            }
            // (stage R, MCLA_DBG_VEHCON: every contact with a vehicle as it was solved, the numbers' bits -
            // tick, the vehicle's level index, depth, friction, elasticity, normal, A's point, B's point, impulse)
            if vehcon() {
                for (l, t) in links.iter().zip(touches.iter()) {
                    let other = [l.a, l.b].into_iter().find_map(|w| if let Some(Who::Obj(o)) = w { self.objs[o].veh } else { None });
                    if let Some(vh) = other {
                        let h = |v: Vec3| format!("{:08x} {:08x} {:08x}", v.x.to_bits(), v.y.to_bits(), v.z.to_bits());
                        eprintln!("VEHCON {} {} {:08x} {:08x} {:08x} {} {} {} {}", REPLAY_TICK.load(std::sync::atomic::Ordering::Relaxed), vh.index, t.depth.to_bits(), t.friction.to_bits(), t.elasticity.to_bits(), h(t.normal), h(t.a.point), h(t.b.point), h(t.impulse));
                    }
                }
            }
            // (the contacts' numbers back into what is kept)
            for (l, t) in links.iter().zip(touches.iter()) {
                if let Some((k, i)) = l.from {
                    if let Some(s) = self.manis.get_mut(k).and_then(|m| m.slots.get_mut(i)) {
                        s.t.kept = t.kept;
                        s.t.impulse = t.impulse;
                    }
                }
            }
            // (each run as the callbacks find it, after the breaking pass: the first row's other object - re-pointed to
            // a piece if it broke off - active or not (level state 0: a body, not asleep), and whether a row is on a
            // part of the car that is not a wheel, and whether a row of it has an impulse from the solve)
            let side_of = |n: usize| -> Option<(Who, u16)> {
                match src[n] {
                    RowSrc::City(r) => Some((Who::City, city_part.get(r).copied().unwrap_or(0) as u16)),
                    RowSrc::Mani(k) => {
                        let m = &self.manis[k];
                        if m.a == Who::Car {
                            Some((m.b, m.part_a))
                        } else if m.b == Who::Car {
                            Some((m.a, m.part_b))
                        } else {
                            None
                        }
                    }
                }
            };
            let car_runs = car_runs_at
                .iter()
                .filter(|r| !r.is_empty())
                .map(|r| {
                    let first_active = side_of(r[0]).is_some_and(|(o, _)| matches!(o, Who::Obj(o) if self.objs.get(o).is_some_and(|x| x.body.is_some() && !x.dormant)));
                    let hull = r.iter().any(|&n| side_of(n).is_some_and(|(_, part)| part == 0));
                    let any = r.iter().any(|&n| row_of[n] != usize::MAX && touches[row_of[n]].impulse != Vec3::ZERO);
                    if super::vehsim::dbg_flag("MCLA_DBG_STUCK") { eprintln!("STUCKRUN tick {} first {:?} body {:?} dormant {:?} rows {}", REPLAY_TICK.load(std::sync::atomic::Ordering::Relaxed), side_of(r[0]), side_of(r[0]).and_then(|(o, _)| if let Who::Obj(o) = o { self.objs.get(o).map(|x| x.body.is_some()) } else { None }), side_of(r[0]).and_then(|(o, _)| if let Who::Obj(o) = o { self.objs.get(o).map(|x| x.dormant) } else { None }), r.len()); }
                    (first_active, hull, any)
                })
                .collect();
            return IslandOut { impulse: given[0].0, turning: given[0].1, touches, links, bodies, owners, solved, by_b, off_a, off_b, car_runs };
        }
    }

    /// The prop instances' impact report after the island's solve (`sub_82489EE8`, slot 25; `prop_hit_fx_rule.md` 2a):
    /// per run of an instance's rows, as A and as B, the row with the largest impulse (the contact's +12, I) gives
    /// the point on the prop, the normal turned to the prop's side and the closing speed of the two bodies' velocities;
    /// when it is over 0.1 the type's events whose least speed it reaches are started at the point (+ the event's
    /// offset, world axes).
    pub fn impact_reports(&mut self, world: &LooseWorld, out: &IslandOut) {
        for as_a in [true, false] {
            let mut done: Vec<usize> = Vec::new();
            for l in out.links.iter() {
                let Some(Who::Obj(o)) = (if as_a { l.a } else { l.b }) else { continue };
                if done.contains(&o) {
                    continue;
                }
                done.push(o);
                let ob = &self.objs[o];
                if ob.veh.is_some() || ob.amb.is_some() || world.types[ob.ty].type_events.is_empty() {
                    continue;
                }
                let mut best: Option<(f32, usize)> = None;
                for (j, m) in out.links.iter().enumerate() {
                    if (if as_a { m.a } else { m.b }) == Some(Who::Obj(o)) && best.map_or(true, |b| out.touches[j].impulse.length() > b.0) {
                        best = Some((out.touches[j].impulse.length(), j));
                    }
                }
                let Some((_, j)) = best else { continue };
                let t = &out.touches[j];
                let (me, other, n) = if as_a { (&t.a, &t.b, t.normal) } else { (&t.b, &t.a, t.back) };
                let vp = me.body.and_then(|k| out.bodies.get(k)).map_or(Vec3::ZERO, |b| b.vel);
                let vo = other.body.and_then(|k| out.bodies.get(k)).map_or(Vec3::ZERO, |b| b.vel + b.ang.cross(me.point - b.at));
                let speed = -n.dot(vp - vo);
                if speed > 0.1 {
                    for e in &world.types[ob.ty].type_events {
                        if speed >= e.min_speed {
                            self.fx_events.push(FxPlay { effect: e.effect.clone(), at: FxAt::World(me.point + v(e.offset)), vel: Vec3::ZERO });
                        }
                    }
                }
            }
        }
    }

    /// Start the events of a break set on object `w`, following the matrix of `part` (none: the instance's).
    fn fx_follow(&mut self, w: usize, set: &[mcla_formats::rage::fragphys::PropEvent], part: Option<u16>) {
        for e in set {
            self.fx_events.push(FxPlay { effect: e.effect.clone(), at: FxAt::Follow { obj: w, part, offset: v(e.offset) }, vel: Vec3::ZERO });
        }
    }

    /// The break-from-root sets of group `g` and, below it, of its sub-groups that have not broken (`sub_824D1B68`).
    fn fx_root(&mut self, t: &FragType, w: usize, g: usize, absent: u64) {
        let Some(gr) = t.groups.get(g) else { return };
        let part = fx_part(t, g);
        for c in gr.first_child as usize..gr.first_child as usize + gr.children as usize {
            if let Some(ch) = t.children.get(c) {
                self.fx_follow(w, &ch.break_root_events, part);
            }
        }
        for sg in gr.first_group as usize..gr.first_group as usize + gr.groups as usize {
            if sg < 64 && absent & (1 << sg) == 0 {
                self.fx_root(t, w, sg, absent);
            }
        }
    }

    /// The matrix an event effect follows, with the offset it is placed at (`FxAt::Follow`).
    pub fn fx_matrix(&self, world: &LooseWorld, obj: usize, part: Option<u16>) -> Option<[Vec3; 4]> {
        let o = self.objs.get(obj)?;
        match part {
            None => Some(o.matrix),
            // (the bone as the unposed skeleton has it, carried by the instance's matrix; a prop that has not been
            // posed has every bone where its skeleton puts it - the hydrant's are both the identity)
            Some(b) => Some(world.types[o.ty].bones.get(b as usize).map_or(o.matrix, |m| super::gjk::world_matrix(&m4(m), &o.matrix))),
        }
    }

    /// 0x824900A8 -> 0x8248F320: an instance breaks at `group` (255: it goes as a whole). Gives
    /// the new instance and the groups it took, if a piece came off.
    fn break_apart(&mut self, world: &LooseWorld, w: usize, group: u8) -> Option<(usize, u64)> {
        let ty = self.objs[w].ty;
        let t = &world.types[ty];
        let (max_speed, max_spin) = (500.0f32, f32::from_bits(0x40c9_0fdb));
        let was_hit = self.objs[w].hit;
        if self.objs[w].entry.is_none() {
            if t.flags & 1 == 0 {
                // (it goes as it is, with the file's archetype: child 0's break set on the instance's matrix, 0x824900A8)
                if let Some(c0) = t.children.first() {
                    self.fx_follow(w, &c0.break_events, None);
                }
                let obj = &mut self.objs[w];
                obj.hit = true;
                if obj.body.is_none() {
                    let a = &t.archetype;
                    let e = Entry { absent: 0, age: 0.0, cg: v(t.cg), mass: a.mass, inv_mass: a.inv_mass, inertia: v(a.inertia), inv_inertia: v(a.inv_inertia), ..Default::default() };
                    obj.body = Some(PBody::new(&obj.matrix, e.cg, &e, a.max_speed, a.max_spin));
                    (obj.sleep, obj.dormant) = (Sleep::reset(), false);
                }
                return None;
            }
            // (the prop gets an entry that holds every group, 0x8248A3B0)
            let mut e = Entry::default();
            let mass = entry_mass(t, 0);
            entry_refresh(t, &mut e, mass);
            self.objs[w].entry = Some(e);
            // (the entry has an instance of its own, which a prop's does not use)
            self.entry_instance();
        }
        if group == 255 {
            // (the whole instance comes loose: the break-from-root sets of the root groups, 0x82490284)
            let absent = self.objs[w].entry.map_or(0, |e| e.absent);
            for g in 0..t.root_groups as usize {
                self.fx_root(t, w, g, absent);
            }
            let obj = &mut self.objs[w];
            obj.hit = true;
            if obj.body.is_none() {
                let e = obj.entry.unwrap();
                obj.body = Some(PBody::new(&obj.matrix, e.cg, &e, max_speed, max_spin));
                (obj.sleep, obj.dormant) = (Sleep::reset(), false);
            }
            return None;
        }
        let parent = self.objs[w].entry.unwrap();
        let bits = subtree(t, group as usize) & !parent.absent;
        let all: u64 = (0..t.groups.len()).fold(0, |b, g| b | 1 << g);
        // (something must stay behind, else it goes as a whole)
        if (all & !parent.absent) & !bits == 0 {
            for g in 0..t.root_groups as usize {
                self.fx_root(t, w, g, parent.absent);
            }
            let obj = &mut self.objs[w];
            obj.hit = true;
            if obj.body.is_none() {
                obj.body = Some(PBody::new(&obj.matrix, parent.cg, &parent, max_speed, max_spin));
                (obj.sleep, obj.dormant) = (Sleep::reset(), false);
            }
            return None;
        }
        // (the new entry: the parent's state, less everything outside the group's own tree)
        let mut e = Entry { absent: all & !bits, age: parent.age, from: self.objs[w].inst.or(parent.from), ..Default::default() };
        let mass = entry_mass(t, e.absent);
        entry_refresh(t, &mut e, mass);
        let matrix = self.objs[w].matrix;
        let mut body = PBody::new(&matrix, e.cg, &e, max_speed, max_spin);
        // (broken off a moving parent it takes the parent's motion with it, 0x825A5AE8 / 0x825A5168)
        if let Some(pb) = self.objs[w].body {
            let pos = xf(&matrix, parent.cg);
            let r = matrix[3] - pos;
            let vel = cross(pb.w, r) + pb.v;
            body.v = vel;
            body.p = vel * body.mass;
            body.w = pb.w;
            body.l = body.momentum_of(pb.w);
        }
        // (the events of the break, 0x8248F320: with the instance not broken before, the break-from-root sets of the
        // group's subtree, then the break sets of the group's own children, on the part that stays)
        if !was_hit {
            self.fx_root(t, w, group as usize, parent.absent);
        }
        if let Some(gr) = t.groups.get(group as usize) {
            let part = fx_part(t, group as usize);
            for c in gr.first_child as usize..gr.first_child as usize + gr.children as usize {
                if let Some(ch) = t.children.get(c) {
                    self.fx_follow(w, &ch.break_events, part);
                }
            }
        }
        self.pieces_made += 1;
        let n = self.pieces_made;
        let new = self.objs.len();
        let (inst_addr, index) = (self.entry_instance(), self.new_index());
        self.objs.push(Obj { ty, inst: None, matrix, entry: Some(e), hit: true, body: Some(body), level: index.unwrap_or(rank('l', new).unwrap_or(if hint("pb", n) { 2_000_000 + n as i64 } else { -1000 - n as i64 })), addr: inst_addr.unwrap_or(rank('a', new).unwrap_or(if hint("pafter", n) { 2_000_000 + n as i64 } else { -1000 - n as i64 })), veh: None, sleep: Sleep::reset(), dormant: false, amb: None });
        if dbg() {
            eprintln!("LOOSENEW tick {} obj {new} level {} addr {:x}", self.tick, self.objs[new].level, self.objs[new].addr);
        }
        // (the parent loses the groups, 0x824A9458 and 0x824A9130; a body keeps its velocities)
        let mut pe = parent;
        pe.absent |= bits;
        let rest = entry_mass(t, pe.absent);
        entry_refresh(t, &mut pe, rest);
        let obj = &mut self.objs[w];
        obj.entry = Some(pe);
        if let Some(b) = obj.body.as_mut() {
            b.rows = [obj.matrix[0], obj.matrix[1], obj.matrix[2]];
            b.at = xf(&obj.matrix, pe.cg);
            b.mass = pe.mass;
            b.inv_mass = pe.inv_mass;
            b.inertia = pe.inertia;
            b.inv_inertia = pe.inv_inertia;
            b.p = b.v * b.mass;
            b.l = b.momentum_of(b.w);
        }
        if dbg() {
            eprintln!("LOOSEPIECE tick {} from obj {w}: obj {new} groups {bits:b} mass {:?} cg {:?} inertia {:?}; the rest mass {:?} cg {:?}", self.tick, e.mass, e.cg, e.inertia, pe.mass, pe.cg);
        }
        Some((new, bits))
    }

    /// The island's push solve (0x82591108) and what it leaves on the loose bodies; gives the
    /// car's push.
    pub fn push_car_island(&mut self, out: &mut IslandOut, car: &Body, car_start: Pushed, dt: f32) -> Pushed {
        // (the bodies as the integration left them: only their inverse masses and matrices
        // enter - and those have not changed)
        let mut bodies = out.bodies.clone();
        bodies[0] = *car;
        let mut pushes = vec![Pushed::default(); bodies.len()];
        pushes[0] = car_start;
        for (k, o) in out.owners.iter().enumerate() {
            let b = self.objs[*o].body.unwrap();
            pushes[k + 1] = Pushed { push: b.push, turn: b.turn, deep: false };
        }
        solver::island_push(&bodies, &mut out.touches, &mut out.solved, dt, 1.0 / dt, &mut pushes);
        for (k, o) in out.owners.iter().enumerate() {
            let b = self.objs[*o].body.as_mut().unwrap();
            b.push = pushes[k + 1].push;
            b.turn = pushes[k + 1].turn;
            if pushes[k + 1].deep && !b.deep {
                b.deep = true;
                b.deep_count = b.deep_count.saturating_add(1).min(254);
            }
        }
        for (l, t) in out.links.iter().zip(out.touches.iter()) {
            if let Some((k, i)) = l.from {
                if let Some(s) = self.manis.get_mut(k).and_then(|m| m.slots.get_mut(i)) {
                    s.t.pushed = t.pushed;
                }
            }
        }
        pushes[0]
    }
}

// ---------------------------------------------------------------------------------------------
// The loose bodies' own tick (p1/body_decode.md): the move, gravity and damping ahead of the
// collide (0x8259F300), the integration after the solves (0x8259BE30), the push applied
// (0x8259E920), and the entries' ages (0x824AB680).

const GRAVITY: f32 = f32::from_bits(0xc11c_cccd);

impl LooseState {
    /// The groups of manifolds that hang together through an object that can move or be moved
    /// (the city joins nothing): the car's - empty if nothing loose touches it - and the rest.
    fn islands(&self) -> (Vec<usize>, Vec<Vec<usize>>) {
        if !super::vehsim::dbg_flag("MCLA_DBG_RA2") {
            let live = |m: &Mani| m.slots.iter().any(|s| s.enabled);
            let mut car = Vec::new();
            let mut rest: Vec<(usize, Vec<usize>)> = Vec::new();
            for (k, m) in self.manis.iter().enumerate() {
                if m.island == 0 {
                    car.push(k);
                } else {
                    match rest.iter().position(|x| x.0 == m.island) {
                        Some(i) => rest[i].1.push(k),
                        None => rest.push((m.island, vec![k])),
                    }
                }
            }
            // (an island's rows count the contacts that are not enabled too; one with none enabled is not solved)
            if !car.iter().any(|k| live(&self.manis[*k])) {
                car.clear();
            }
            rest.retain(|x| x.1.iter().any(|k| live(&self.manis[*k])));
            return (car, rest.into_iter().map(|x| x.1).collect());
        }
        let n = self.objs.len() + 1;
        let mut root: Vec<usize> = (0..n).collect();
        fn find(root: &mut Vec<usize>, mut x: usize) -> usize {
            while root[x] != x {
                root[x] = root[root[x]];
                x = root[x];
            }
            x
        }
        let node = |w: Who| match w {
            Who::Car => Some(0),
            Who::Obj(o) => Some(o + 1),
            Who::City => None,
        };
        let live = |m: &Mani| m.slots.iter().any(|s| s.enabled);
        for m in self.manis.iter().filter(|m| live(m)) {
            if let (Some(a), Some(b)) = (node(m.a), node(m.b)) {
                let (ra, rb) = (find(&mut root, a), find(&mut root, b));
                if ra != rb {
                    root[rb.max(ra)] = rb.min(ra);
                }
            }
        }
        let car_root = find(&mut root, 0);
        let mut car = Vec::new();
        let mut rest: Vec<(usize, Vec<usize>)> = Vec::new();
        for (k, m) in self.manis.iter().enumerate() {
            if !live(m) {
                continue;
            }
            let r = find(&mut root, node(m.a).unwrap_or(0));
            if r == car_root {
                car.push(k);
            } else {
                match rest.iter().position(|x| x.0 == r) {
                    Some(i) => rest[i].1.push(k),
                    None => rest.push((r, vec![k])),
                }
            }
        }
        // (the car's own island holds something loose only if a manifold of the car is in it)
        if !car.iter().any(|k| self.manis[*k].a == Who::Car || self.manis[*k].b == Who::Car) {
            for k in car.drain(..) {
                let r = node(self.manis[k].a).unwrap_or(0);
                match rest.iter().position(|x| x.0 == r) {
                    Some(i) => rest[i].1.push(k),
                    None => rest.push((r, vec![k])),
                }
            }
        }
        (car, rest.into_iter().map(|x| x.1).collect())
    }

    /// For whoever draws the props (stage P4): the prop instances that have lost groups - the instance's number in
    /// the file's order and the bit set of the groups gone - and every piece or loose prop that has a body or lies asleep (`dormant`): its
    /// type, the groups it does NOT hold, and its instance's matrix (rows and place).
    pub fn drawn(&self) -> (Vec<(u32, u64)>, Vec<(usize, u64, [Vec3; 4])>) {
        let broken = self.objs.iter().filter_map(|o| Some((o.inst?, o.entry?.absent))).filter(|x| x.1 != 0).collect();
        let moving = self.objs.iter().filter(|o| o.body.is_some() || o.dormant).map(|o| (o.ty, o.entry.map_or(0, |e| e.absent), o.matrix)).collect();
        (broken, moving)
    }

    /// The wake of an island's inactive objects (0x82590A60 -> 0x8258D3F8): every object of every manifold `ids` - one
    /// whose contacts are all disabled too - when the island has a prop or piece in it.
    fn wake_island(&mut self, world: &LooseWorld, ids: &[usize]) {
        let objs: Vec<usize> = ids.iter().flat_map(|&k| [self.manis[k].a, self.manis[k].b]).filter_map(|w| if let Who::Obj(o) = w { Some(o) } else { None }).collect();
        if dbg() {
            eprintln!("LOOSEWAKEISLE tick {} {:?}", self.tick, objs.iter().map(|&o| (o, self.breakable(world, o), self.objs[o].dormant, self.objs[o].hit, self.objs[o].body.is_some())).collect::<Vec<_>>());
        }
        // (every island with a prop or piece in it goes to the breaking pass - gjkfix part 10: contact limit +60 = 0,
        // island +75 set for every island - not only one with something left to break: V s_rtv s473, 22360 a piece of
        // one group woken by the car's island)
        if objs.iter().any(|&o| self.objs[o].veh.is_none()) {
            for o in objs {
                self.wake_obj(world, o);
            }
        }
    }

    /// The car's island with no loose contact enabled in it: the port solves the car alone (or not at all), the original
    /// has the car's loose manifolds in the island and wakes what they hold, solved or not (V s_rtv s473: the car's wheel
    /// 3 1.1 cm from asleep piece 22360, with car x city contacts; s496: wheel 1, none - contact flags 0x4d both times).
    pub fn wake_car_island(&mut self, world: &LooseWorld) {
        let ids: Vec<usize> = (0..self.manis.len()).filter(|&k| self.manis[k].island == 0).collect();
        self.wake_island(world, &ids);
    }

    /// The manifolds of the car's island (empty: the car's island is the car and the city).
    pub fn car_island(&self) -> Vec<usize> {
        self.islands().0
    }

    /// Every island the car is not in: its solve, its breaking pass, and its push solve.
    pub fn step_rest(&mut self, world: &LooseWorld, car_in: &CarIn, dt: f32) {
        let asleep = self.islands_asleep();
        for ids in self.islands().1 {
            if ids.iter().any(|k| asleep.contains(&self.manis[*k].island)) {
                continue;
            }
            let mut none = Body { at: Vec3::ZERO, rows: [Vec3::X, Vec3::Y, Vec3::Z], vel: Vec3::ZERO, ang: Vec3::ZERO, inv_mass: 0.0, inv_moments: Vec3::ZERO, force: Vec3::ZERO, torque: Vec3::ZERO };
            let (mut p, mut l) = (Vec3::ZERO, Vec3::ZERO);
            let mut dynm = CarDyn { p: &mut p, l: &mut l, mass: 0.0, inverse: Vec3::ZERO, rows: [Vec3::X, Vec3::Y, Vec3::Z] };
            let mut out = self.solve_car_island(world, car_in, &mut none, &mut dynm, &[], &[], &[], &ids, dt);
            self.push_car_island(&mut out, &none, Pushed::default(), dt);
        }
    }

    /// The islands no car is in, for a level shared by the cars (the game: once a tick, after every car).
    pub fn step_rest_alone(&mut self, world: &LooseWorld, dt: f32) {
        static NONE: std::sync::OnceLock<HullShape> = std::sync::OnceLock::new();
        let hull = NONE.get_or_init(|| HullShape::new(vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z], &[[0, 1, 2], [0, 1, 3], [0, 2, 3], [1, 2, 3]]));
        let m = [Vec3::X, Vec3::Y, Vec3::Z, Vec3::ZERO];
        let car = CarIn { hull, hull_cg: Vec3::ZERO, hull_margin: 0.04, now: m, last: m, speed: 0.0, bound: (Vec3::ZERO, Vec3::ZERO), wheels: Vec::new(), vel: Vec3::ZERO, touch: (0.0, 0.0), wiped: false };
        self.step_rest(world, &car, dt);
    }

    /// (replay diagnostics, `MCLA_REPLAY_PIECES=<file>`: every loose body put into the recording's state before each
    /// update - lines "<update> <level index>" and the pool body's place, rows, v, w, P, L, force, torque, the two
    /// impulse sums, push, turn, its "last" matrix (12) and whether it is set, and the instance's matrix (12), as
    /// work/proppath/mkpieces.py writes them from the OBJ records - so that each tick is one step from the original's)
    fn replay_pieces(&mut self) {
        static P: std::sync::OnceLock<HashMap<(i64, i64), Vec<f32>>> = std::sync::OnceLock::new();
        let p = P.get_or_init(|| {
            let Some(path) = std::env::var_os("MCLA_REPLAY_PIECES") else { return HashMap::new() };
            std::fs::read_to_string(path).unwrap_or_default().lines().filter_map(|l| {
                let w: Vec<&str> = l.split_whitespace().collect();
                let (k, lev) = (w.first()?.parse().ok()?, w.get(1)?.parse().ok()?);
                Some(((k, lev), w[2..].iter().filter_map(|x| x.parse().ok()).collect()))
            }).collect()
        });
        if p.is_empty() {
            return;
        }
        let k = REPLAY_K.load(std::sync::atomic::Ordering::Relaxed);
        for o in self.objs.iter_mut() {
            let Some(b) = o.body.as_mut() else { continue };
            let Some(q) = p.get(&(k, o.level)).filter(|q| q.len() >= 70) else { continue };
            let v3 = |i: usize| Vec3::new(q[i], q[i + 1], q[i + 2]);
            b.at = v3(0);
            b.rows = [v3(3), v3(6), v3(9)];
            b.v = v3(12);
            b.w = v3(15);
            b.p = v3(18);
            b.l = v3(21);
            b.force = v3(24);
            b.torque = v3(27);
            b.jf = v3(30);
            b.jt = v3(33);
            b.push = v3(36);
            b.turn = v3(39);
            b.last = [v3(42), v3(45), v3(48), v3(51)];
            b.last_set = q[54] != 0.0;
            o.matrix = [v3(55), v3(58), v3(61), v3(64)];
            b.deep = q[67] != 0.0;
            b.deep_count = q[68] as u8;
            b.moves = q[69] as u32;
        }
    }

    /// The instance's matrix from the body (0x8259D1E8).
    fn matrix_from_body(&mut self, world: &LooseWorld, o: usize) {
        let cg = self.cg_of(world, o);
        let obj = &mut self.objs[o];
        if let Some(b) = obj.body.as_ref() {
            obj.matrix = [b.rows[0], b.rows[1], b.rows[2], ex::origin_of(b.at, &b.rows, cg)];
        }
    }

    /// Ahead of the collide (0x8259F300): what the push solve left is applied, the body is moved
    /// with the velocities the tick came in with, and gravity and the type's damping go into the
    /// tick's force sums.
    pub fn pre_collide(&mut self, world: &LooseWorld, dt: f32) {
        let rs = super::gjk::rsqrt_as_original;
        self.replay_pieces();
        self.pieces_wake(dt);
        self.bodies_wake(dt);
        self.sync_others();
        for o in 0..self.objs.len() {
            let Some(mut b) = self.objs[o].body else { continue };
            let veh = self.objs[o].veh;
            // (a live car is moved by its own step; `set_vehicles` hands it in already moved - an ambient
            // car's body is the level's)
            if veh.is_some() && self.live && self.objs[o].amb.is_none() {
                continue;
            }
            let (gravity, damping) = match veh {
                Some(vh) => (vh.gravity, VEH_DAMPING),
                None => {
                    let t = &world.types[self.objs[o].ty];
                    (t.archetype.gravity, t.damping)
                }
            };
            b.prev = self.objs[o].matrix;
            // (0x8259E790 - normally nothing is pending)
            b.at = b.at + b.push;
            b.push = Vec3::ZERO;
            if solver::turn_applies(b.turn) {
                b.rows = solver::rows_pushed(&b.rows, b.turn);
                b.turn = Vec3::ZERO;
            }
            // (0x8259E768)
            b.at = b.v * dt + b.at;
            let turn = b.w * dt;
            if dot3(turn, turn) > f32::from_bits(0x2b8c_bccc) {
                b.rows = ex::rows_turned(&b.rows, turn);
                (b.w, b.l) = capped(ex::spin_of(b.l, &b.rows, b.inv_inertia), b.l, b.max_spin);
            }
            self.objs[o].body = Some(b);
            self.matrix_from_body(world, o);
            // (gravity, 0x8259D7F0: a force)
            let g = Vec3::new(0.0 * gravity, GRAVITY * gravity, 0.0 * gravity);
            b.force = b.force + g * b.mass;
            // (damping, 0x8259D880: in the body's axes, of an entry's archetype - and of the file's own
            // archetype for a prop that went loose whole (phArchetypeDamp, vtable[17] true: V s_rtv 11286,
            // 25.5 kg, in flight from s386 its speed 0.269 % less a tick on every axis, the type's 0.02 /
            // 0.1 / 0.02 at 2.7 m/s). Not the torn bumper's plain phArchetypePhys.)
            let file_type = veh.is_none() && world.types[self.objs[o].ty].off != usize::MAX;
            if self.objs[o].entry.is_some() || veh.is_some() || file_type {
                let inv_dt = recip(dt);
                let cols = [Vec3::new(b.rows[0].x, b.rows[1].x, b.rows[2].x), Vec3::new(b.rows[0].y, b.rows[1].y, b.rows[2].y), Vec3::new(b.rows[0].z, b.rows[1].z, b.rows[2].z)];
                let slow = |x: Vec3, c: Vec3, lin: Vec3, sq: Vec3| {
                    let l = Vec3::new(dot3(b.rows[0], x), dot3(b.rows[1], x), dot3(b.rows[2], x));
                    let l2 = dot3(l, l);
                    let sp = if l2 == 0.0 { 0.0 } else { l2 * rs(l2) };
                    let u = if l2 > f32::from_bits(0x3727_c5ac) { l * rs(l2) } else { Vec3::ZERO };
                    let mut a = Vec3::ZERO - ((u * c) + (((l * sp) * sq) + (l * lin)));
                    let n = a * dt + l;
                    if 0.0 >= dot3(n, l) {
                        a = -(l * inv_dt - Vec3::ZERO);
                    }
                    a
                };
                let d = &damping;
                let fl = slow(b.v, v(d[0]), v(d[1]), v(d[2])) * b.mass;
                b.force = b.force + Vec3::new(dot3(cols[0], fl), dot3(cols[1], fl), dot3(cols[2], fl));
                let tl = slow(b.w, v(d[3]), v(d[4]), v(d[5])) * b.inertia;
                b.torque = b.torque + Vec3::new(dot3(cols[0], tl), dot3(cols[1], tl), dot3(cols[2], tl));
            }
            b.moves += 1;
            if b.moves > 32 {
                b.rows = ex::rows_squared(&b.rows);
                b.moves = 0;
            }
            self.objs[o].body = Some(b);
            self.matrix_from_body(world, o);
            if dbg() {
                eprintln!("LOOSEBODY tick {} obj {o} at {:?} v {:?} w {:?} origin {:?} up {:?}", self.tick + 1, b.at, b.v, b.w, self.objs[o].matrix[3], b.rows[1]);
            }
        }
    }

    /// After the solves: the forces and impulses into the momenta (0x8259BE30), the push solve's
    /// answer into the pose (0x8259E920), and every entry a tick older (0x824AB680).
    pub fn end_tick(&mut self, world: &LooseWorld, dt: f32) {
        self.end_tick_with(world, dt, None)
    }

    /// `end_tick`, with the city an ambient car's wheels probe (slot 14, after the integration and
    /// before the push goes into the pose: sub_826FA0F8, then the base 0x8259E920).
    pub fn end_tick_with(&mut self, world: &LooseWorld, dt: f32, city: Option<&super::collision::WorldCollider>) {
        self.pieces_sleep(dt);
        self.bodies_sleep(dt);
        for o in 0..self.objs.len() {
            let Some(mut b) = self.objs[o].body else { continue };
            if self.objs[o].veh.is_some() && self.live && self.objs[o].amb.is_none() {
                continue;
            }
            b.jf = b.force * dt + b.jf;
            b.jt = b.torque * dt + b.jt;
            b.force = Vec3::ZERO;
            b.torque = Vec3::ZERO;
            if b.jf != Vec3::ZERO {
                b.p = b.p + b.jf;
                (b.v, b.p) = capped(b.p * b.inv_mass, b.p, b.max_speed);
                b.jf = Vec3::ZERO;
            }
            if b.jt != Vec3::ZERO {
                b.l = b.l + b.jt;
                (b.w, b.l) = capped(ex::spin_of(b.l, &b.rows, b.inv_inertia), b.l, b.max_spin);
                b.jt = Vec3::ZERO;
            }
            if let Some(city) = city {
                let m = self.objs[o].matrix;
                if let Some(a) = self.objs[o].amb.as_mut() {
                    ambient_wheels(a, &mut b, &m, city, dt);
                }
            }
            // (0x8259E790, then the instance's matrix and the collide's "last" one)
            b.at = b.at + b.push;
            b.push = Vec3::ZERO;
            if solver::turn_applies(b.turn) {
                b.rows = solver::rows_pushed(&b.rows, b.turn);
                b.turn = Vec3::ZERO;
            }
            self.objs[o].body = Some(b);
            self.matrix_from_body(world, o);
            let now = self.objs[o].matrix;
            let b = self.objs[o].body.as_mut().unwrap();
            if !b.deep {
                b.last = now;
                b.last_set = true;
                b.deep_count = 0;
            } else {
                if !b.last_set || b.deep_count > 8 {
                    b.last = now;
                }
                b.deep = false;
            }
        }
        // (the entries' ages; a quarter of a second after the first hit the centre of gravity
        // goes back where the parts have it)
        for o in 0..self.objs.len() {
            let ty = self.objs[o].ty;
            let Some(mut e) = self.objs[o].entry else { continue };
            let t = &world.types[ty];
            let age = e.age;
            e.age = age + dt;
            if age < 0.25 && e.age >= 0.25 && v(t.cg_shift) != Vec3::ZERO {
                let mass = entry_mass(t, e.absent);
                entry_refresh(t, &mut e, mass);
                let obj = &mut self.objs[o];
                if let Some(b) = obj.body.as_mut() {
                    b.rows = [obj.matrix[0], obj.matrix[1], obj.matrix[2]];
                    b.at = xf(&obj.matrix, e.cg);
                    b.mass = e.mass;
                    b.inv_mass = e.inv_mass;
                    b.inertia = e.inertia;
                    b.inv_inertia = e.inv_inertia;
                    b.p = b.v * b.mass;
                    b.l = b.momentum_of(b.w);
                }
            }
            self.objs[o].entry = Some(e);
        }
        if dbg() {
            let h = |v: Vec3| format!("{:08x} {:08x} {:08x}", v.x.to_bits(), v.y.to_bits(), v.z.to_bits());
            for o in self.objs.iter() {
                if let Some(b) = o.body.as_ref() {
                    eprintln!("LOOSEEND k {} level {} age{:?}/{:08x} at {} v {} w {} p {} l {} m {} {} {} {}", REPLAY_K.load(std::sync::atomic::Ordering::Relaxed), o.level, o.entry.map(|e| e.age), dt.to_bits(), h(b.at), h(b.v), h(b.w), h(b.p), h(b.l), h(o.matrix[0]), h(o.matrix[1]), h(o.matrix[2]), h(o.matrix[3]));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Stage R (agent D): another VEHICLE as a body of the car's island (flag `MCLA_DBG_RD1` = none).
// An ambient traffic vehicle (instance class 0x820C4314, class number 6, collider class
// 0x820C4114) is one polyhedron bound (type 4, `resources/ambients/<name>/*.r155`) on a body of
// 600 kg whose archetype (phArchetypeDamp, made by 0x826E3EC8) has gravity factor 1.2 and the
// damping (C, V, V2) = (0.01, 0.01, 0.001) for moving and for turning. Its own driving (the AI,
// its ray suspension 0x826FA0F8 that leaves its forces in the body's sums for the next step) is
// the session's: a replay hands the body's state before every tick (`MCLA_REPLAY_OTHERS`, written
// by veh_diff.py from the recording's OBJ records); the move, gravity, damping, the contacts
// with the car's hull, the car's callback 0x82332D30, the island's solve and the push are this
// port's own.

/// What a vehicle object is beyond a prop's `Obj`.
#[derive(Clone, Copy, Debug)]
pub struct VehObj {
    pub shape: &'static HullShape,
    /// an ambient car's composite's second part (a semi trailer's: the file's part matrices are all
    /// the identity)
    pub shape2: Option<&'static HullShape>,
    /// instance vtable[1]: 6 ambient traffic, 8 an mcCar
    pub class: u8,
    /// archetype +48
    pub gravity: f32,
    /// bound +80 as the running game has it
    pub cg: Vec3,
    /// its level index
    pub index: u32,
    /// it is in this tick's list as an active object
    pub here: bool,
    /// its mcCarDamage +20, the "damage inflicted" factor a car it hits takes its blow times (0x82342EA8)
    pub inflict: f32,
}

impl VehObj {
    /// its bound's part `p`
    pub fn part(&self, p: u16) -> &'static HullShape {
        if p == 1 { self.shape2.unwrap_or(self.shape) } else { self.shape }
    }

    pub fn parts(&self) -> u16 {
        if self.shape2.is_some() { 2 } else { 1 }
    }
}

#[derive(Clone, Debug)]
struct OtherVeh {
    index: u32,
    addr: u32,
    class: u8,
    state: u32,
    model: String,
    matrix: [Vec3; 4],
    body: PBody,
    gravity: f32,
    cg: Vec3,
}

/// A contact the pair car x vehicle kept from the tick before (one-step replays only).
#[derive(Clone, Copy, Debug)]
struct OtherKept {
    index: u32,
    local_a: Vec3,
    local_b: Vec3,
    normal: Vec3,
    back: Vec3,
    kept: Vec3,
    depth: f32,
    element: u16,
    element_b: u16,
}

pub struct Others {
    mine: u32,
    car_addr: u32,
    ticks: HashMap<usize, Vec<OtherVeh>>,
    kept: HashMap<usize, Vec<OtherKept>>,
    shapes: HashMap<String, HullShape>,
    /// the kept contacts are the recording's before every tick (a one-step replay)
    inject: bool,
}

static OTHERS: std::sync::OnceLock<Option<Others>> = std::sync::OnceLock::new();
/// The line of the replay's input the tick in hand is (replay.rs).
pub static REPLAY_TICK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(usize::MAX);

fn vehcon() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var_os("MCLA_DBG_VEHCON").is_some())
}

fn others_only() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var_os("MCLA_REPLAY_OTHERS_ONLY").is_some())
}

fn others() -> Option<&'static Others> {
    OTHERS.get().and_then(|o| o.as_ref())
}

/// `MCLA_REPLAY_OTHERS=<file>`: lines of words, numbers as the hexadecimal bits:
/// `H <car's level index> <car's instance address>`;
/// `V <tick> <level index> <instance address> <class number> <level state> <model> <12 instance matrix>
///    <3 place +256> <9 rows +208> <3 v> <3 w> <3 P> <3 L> <3 F +336> <3 T +352> <mass> <1/mass> <3 I> <3 1/I>
///    <max speed> <max spin> <gravity factor> <3 cg> <moves +664> <12 last matrix +96> <3 push +400> <3 turn +416>`;
/// `K <tick> <level index> <3 local A> <3 local B> <3 normal +80> <3 +96> <3 P +128> <depth> <element A> <element B>`.
/// `hull_of`: the model's bound from the game's files.
pub fn load_others(path: &str, inject: bool, hull_of: &dyn Fn(&str) -> Option<HullShape>) {
    let _ = OTHERS.get_or_init(|| {
        let text = std::fs::read_to_string(path).ok()?;
        let mut o = Others { mine: 0, car_addr: 0, ticks: HashMap::new(), kept: HashMap::new(), shapes: HashMap::new(), inject };
        for line in text.lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            let x = |i: usize| u32::from_str_radix(w.get(i).copied().unwrap_or("0"), 16).unwrap_or(0);
            let f = |i: usize| f32::from_bits(x(i));
            let v3 = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            match w.first().copied() {
                Some("H") => {
                    o.mine = x(1);
                    o.car_addr = x(2);
                }
                Some("V") if w.len() >= 82 => {
                    let model = w[6].to_string();
                    if !o.shapes.contains_key(&model) {
                        match hull_of(&model) {
                            Some(h) => {
                                o.shapes.insert(model.clone(), h);
                            }
                            None => {
                                println!("replay: no bound for the vehicle {model}");
                                continue;
                            }
                        }
                    }
                    let b = 7;
                    let matrix = [v3(b), v3(b + 3), v3(b + 6), v3(b + 9)];
                    let c = b + 12;
                    let body = PBody {
                        at: v3(c),
                        rows: [v3(c + 3), v3(c + 6), v3(c + 9)],
                        v: v3(c + 12),
                        w: v3(c + 15),
                        p: v3(c + 18),
                        l: v3(c + 21),
                        force: v3(c + 24),
                        torque: v3(c + 27),
                        jf: Vec3::ZERO,
                        jt: Vec3::ZERO,
                        push: v3(c + 57),
                        turn: v3(c + 60),
                        last: [v3(c + 45), v3(c + 48), v3(c + 51), v3(c + 54)],
                        prev: [v3(c + 45), v3(c + 48), v3(c + 51), v3(c + 54)],
                        last_set: true,
                        deep: false,
                        deep_count: 0,
                        mass: f(c + 30),
                        inv_mass: f(c + 31),
                        inertia: v3(c + 32),
                        inv_inertia: v3(c + 35),
                        max_speed: f(c + 38),
                        max_spin: f(c + 39),
                        moves: x(c + 44),
                    };
                    o.ticks.entry(x(1) as usize).or_default().push(OtherVeh { index: x(2), addr: x(3), class: x(4) as u8, state: x(5), model, matrix, body, gravity: f(c + 40), cg: v3(c + 41) });
                }
                Some("K") if w.len() >= 21 => {
                    o.kept.entry(x(1) as usize).or_default().push(OtherKept { index: x(2), local_a: v3(3), local_b: v3(6), normal: v3(9), back: v3(12), kept: v3(15), depth: f(18), element: x(19) as u16, element_b: x(20) as u16 });
                }
                _ => {}
            }
        }
        println!("replay: other vehicles in {} ticks, {} bounds{}", o.ticks.len(), o.shapes.len(), if inject { ", kept contacts injected" } else { "" });
        Some(o)
    });
}

/// 0x8259D880's constants on a traffic vehicle's archetype (0x826E3EC8: 0x82002C60 = 0.01, 0x820022B4 = 0.001).
const VEH_DAMPING: [[f32; 3]; 6] = [[0.01; 3], [0.01; 3], [0.001; 3], [0.01; 3], [0.01; 3], [0.001; 3]];

/// A vector made unit as the callback does (vmsum3fp, vrsqrtefp, one step).
fn unit_cb(n: Vec3) -> Vec3 {
    let d = dot3(n, n);
    let e = 1.0 / d.sqrt();
    let e = if e == f32::INFINITY { 0.0 } else { e };
    n * ((1.0 - d * (e * e)) * (0.5 * e) + e)
}

/// The car's callback 0x82332D30 on a contact whose other instance is a vehicle (class 6 or 8):
/// the normal laid flat by how upright both stand; then by where the car is on the other and
/// how it travels - in front of it and head-on (BoundFriction / BoundElasticity, each side's
/// normal the other's axis, both points onto the two centre planes), behind it and the same
/// way (normals likewise, both points the car's on its centre plane), or from the side and
/// into it (the car's point onto its centre plane) - and, the car's own point rewritten on side
/// A, the contact's limit (+60, the point's fourth lane) becomes 0: the island goes through the
/// breaking pass, where nothing breaks. `om`: the other instance's matrix.
fn car_meets_vehicle(car: &CarIn, om: &[Vec3; 4], class: u8, car_is_a: bool, t: &mut Touch) {
    let mut n = if car_is_a { t.normal } else { Vec3::ZERO - t.normal };
    let store = |t: &mut Touch, mine: Vec3, theirs: Vec3| {
        if car_is_a {
            t.normal = mine;
            t.back = theirs;
        } else {
            t.back = mine;
            t.normal = theirs;
        }
    };
    let k = 1.0 - (om[1].y * car.now[1].y).abs();
    if k < 1.0 {
        let mut y2 = n.y * n.y;
        let rest = 1.0 - y2;
        if rest > 0.0 {
            n.y = k * n.y;
            y2 = n.y * n.y;
            let q = ((1.0 - y2) / rest).sqrt();
            n.x *= q;
            n.z *= q;
        }
        let l2 = fma(n.z, n.z, n.x * n.x) + y2;
        if l2 > f32::from_bits(0x3f67_0a3d) && l2 < f32::from_bits(0x3f8d_1eb7) {
            n = unit_cb(n);
            store(t, n, Vec3::ZERO - n);
        }
    }
    let (me, back) = (car.now, car.now[2]);
    let fwd = Vec3::ZERO - back;
    let dir = if fma(car.vel.x, fwd.x, fma(car.vel.z, fwd.z, car.vel.y * fwd.y)) > 0.0 { unit_cb(fwd + car.vel) } else { fwd };
    let ob = om[2];
    let along = |a: Vec3| fma(a.x, ob.x, fma(ob.z, a.z, ob.y * a.y));
    let onto = |m: &[Vec3; 4], p: Vec3| {
        let d = p - m[3];
        let s = -fma(m[0].x, d.x, fma(m[0].y, d.y, m[0].z * d.z));
        m[0] * s + p
    };
    let c45 = ex::guest_cos(f64::from_bits(0x3FE9_21FB_6000_0000)) as f32;
    let side_of = along(n);
    let (mine, theirs) = if car_is_a { (t.a.point, t.b.point) } else { (t.b.point, t.a.point) };
    let (mut mine, mut theirs) = (mine, theirs);
    if side_of < f32::from_bits(0xbf33_3333) {
        if along(dir) > c45 {
            t.friction = car.touch.0;
            t.elasticity = car.touch.1;
            store(t, unit_cb(Vec3::new(-ob.x, n.y, -ob.z)), unit_cb(Vec3::new(-back.x, -n.y, -back.z)));
            mine = onto(om, theirs);
            theirs = onto(&me, mine);
        }
    } else if side_of > f32::from_bits(0x3f33_3333) {
        if along(dir) < -c45 {
            store(t, unit_cb(Vec3::new(ob.x, n.y, ob.z)), unit_cb(Vec3::new(-back.x, -n.y, -back.z)));
            mine = onto(&me, mine);
            theirs = mine;
        }
    } else if fma(dir.x, n.x, fma(dir.z, n.z, dir.y * n.y)) < -c45 {
        mine = onto(&me, mine);
    }
    if car_is_a {
        t.a.point = mine;
        t.b.point = theirs;
    } else {
        t.b.point = mine;
        t.a.point = theirs;
    }
    // (an mcCar partner; the car wiped out, chassis +592 bits 0x10 / 0x04)
    if class == 8 || car.wiped {
        t.friction = car.touch.0;
        t.elasticity = car.touch.1;
    }
}

/// The corner of a box bound the push solve measures a torn bumper's thickness from (type 3: per axis the half size less
/// the margin, signed by the direction in the box's frame; the box is about its origin). None for anything else.
fn piece_core(world: &LooseWorld, obj: &Obj, rows: &[Vec3; 3], n: Vec3) -> Option<Vec3> {
    let t = world.types.get(obj.ty)?;
    if obj.veh.is_some() || t.off != usize::MAX {
        return None;
    }
    let b = t.parts.first()?.bound.as_ref()?;
    let h = (v(b.max) - v(b.min)) * 0.5 - Vec3::splat(b.margin);
    let nl = Vec3::new(dot3(rows[0], n), dot3(rows[1], n), dot3(rows[2], n));
    let s = |d: f32, x: f32| if d >= 0.0 { x } else { -x };
    Some(Vec3::new(s(nl.x, h.x), s(nl.y, h.y), s(nl.z, h.z)))
}

/// The corner of a polyhedron bound the push solve measures a side's thickness from
/// (0x825DCEB8, type 4: the last scanned corner unless an earlier one lies farther along).
fn veh_core(shape: &HullShape, rows: &[Vec3; 3], n: Vec3) -> Vec3 {
    let nl = Vec3::new(dot3(rows[0], n), dot3(rows[1], n), dot3(rows[2], n));
    let c = shape.scanned();
    let Some((&last, firsts)) = c.split_last() else { return Vec3::ZERO };
    let (mut best, mut bd) = (last, dot3(nl, last));
    for p in firsts {
        let d = dot3(nl, *p);
        if d > bd {
            best = *p;
            bd = d;
        }
    }
    best
}

/// A car of the session as another car's step meets it (the game; a replay takes them from its file).
pub struct LiveVeh {
    /// the car's own number (its place among the session's cars)
    pub id: u32,
    pub shape: &'static HullShape,
    /// 8 a car with a driver (mcCar), 6 ambient traffic
    pub class: u8,
    pub gravity: f32,
    /// its body's place in its own frame (the model offset): the instance matrix's origin from the body
    pub cg: Vec3,
    /// its place among the level's indices against the stepping car's (0)
    pub level: i64,
    /// its body moved as the tick's pre-collide moves a body (0x8259E768), its last matrix, and its forces
    pub body: PBody,
    pub inflict: f32,
    /// parked: no body - immovable to the solve
    pub parked: bool,
}

/// A car hull shape kept for the level's whole life (the vehicle objects hold a plain reference).
pub fn static_shape(a: &Arc<HullShape>) -> &'static HullShape {
    use std::sync::Mutex;
    static KEPT: std::sync::OnceLock<Mutex<HashMap<usize, &'static HullShape>>> = std::sync::OnceLock::new();
    let mut m = KEPT.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap();
    *m.entry(Arc::as_ptr(a) as usize).or_insert_with(|| Box::leak(Box::new((**a).clone())))
}

/// What belongs to one car in a shared level: its manifolds (with the city, props, pieces, the other cars)
/// and its pair with the city's place.
#[derive(Clone, Debug, Default)]
pub struct CarPart {
    manis: Vec<Mani>,
    car_city_order: Option<usize>,
}

impl PBody {
    /// A car's body for another car's island: `at` its place (+256), `rows`, velocities, mass and moments,
    /// the matrix the last collide had and the one it has now, the tick's force and torque so far.
    #[allow(clippy::too_many_arguments)]
    pub fn of_car(at: Vec3, rows: [Vec3; 3], v: Vec3, w: Vec3, mass: f32, inertia: Vec3, last: [Vec3; 4], now: [Vec3; 4], force: Vec3, torque: Vec3) -> PBody {
        let mut b = PBody {
            at,
            rows,
            v,
            w,
            p: v * mass,
            l: Vec3::ZERO,
            force,
            torque,
            jf: Vec3::ZERO,
            jt: Vec3::ZERO,
            push: Vec3::ZERO,
            turn: Vec3::ZERO,
            last,
            prev: now,
            last_set: true,
            deep: false,
            deep_count: 0,
            mass,
            inv_mass: recip(mass),
            inertia,
            inv_inertia: Vec3::new(recip(inertia.x), recip(inertia.y), recip(inertia.z)),
            max_speed: 134.1,
            max_spin: 12.566_371,
            moves: 0,
        };
        b.l = b.momentum_of(w);
        b
    }
}

impl LooseState {
    /// The stepping car's own part out of the level (before another car steps in it).
    pub fn take_car(&mut self) -> CarPart {
        let (mine, rest): (Vec<Mani>, Vec<Mani>) = std::mem::take(&mut self.manis).into_iter().partition(|m| m.a == Who::Car || m.b == Who::Car);
        self.manis = rest;
        CarPart { manis: mine, car_city_order: self.car_city_order.take() }
    }

    /// A car's own part into the level, for its step.
    pub fn put_car(&mut self, part: CarPart) {
        self.manis.extend(part.manis);
        self.car_city_order = part.car_city_order;
    }

    /// The other cars for this car's step: each an object with a body (none when parked); those not
    /// listed are not there, and the manifolds with them go.
    pub fn set_vehicles(&mut self, list: Vec<LiveVeh>) {
        self.live = true;
        for o in self.objs.iter_mut() {
            if o.amb.is_some() {
                continue;
            }
            if let Some(vh) = o.veh.as_mut() {
                vh.here = false;
                o.body = None;
            }
        }
        for v in list {
            let i = match self.objs.iter().position(|o| o.amb.is_none() && o.veh.map_or(false, |x| x.index == v.id)) {
                Some(i) => i,
                None => {
                    self.objs.push(Obj { ty: usize::MAX, inst: None, matrix: v.body.prev, entry: None, hit: false, body: None, level: 0, addr: 0, veh: None, sleep: Sleep::reset(), dormant: false, amb: None });
                    self.objs.len() - 1
                }
            };
            let obj = &mut self.objs[i];
            obj.level = v.level;
            obj.addr = v.level;
            obj.matrix = [v.body.rows[0], v.body.rows[1], v.body.rows[2], ex::origin_of(v.body.at, &v.body.rows, v.cg)];
            obj.body = if v.parked { None } else { Some(v.body) };
            obj.veh = Some(VehObj { shape: v.shape, shape2: None, class: v.class, gravity: v.gravity, cg: v.cg, index: v.id, here: true, inflict: v.inflict });
        }
        let objs = &self.objs;
        self.manis.retain(|m| ![m.a, m.b].iter().any(|w| matches!(w, Who::Obj(o) if objs[*o].veh.map_or(false, |x| !x.here))));
    }

    /// What this step's solves gave each other car: its id, the impulse and its turn about its place (for
    /// the momenta), and the push solve's move and turn. Taken: the bodies are handed in afresh each step.
    pub fn vehicle_answers(&mut self) -> Vec<(u32, Vec3, Vec3, Vec3, Vec3)> {
        let mut out = Vec::new();
        for o in self.objs.iter_mut() {
            if o.amb.is_some() {
                continue;
            }
            let (Some(vh), Some(b)) = (o.veh, o.body.as_mut()) else { continue };
            if b.jf != Vec3::ZERO || b.jt != Vec3::ZERO || b.push != Vec3::ZERO || b.turn != Vec3::ZERO {
                out.push((vh.index, b.jf, b.jt, b.push, b.turn));
            }
            b.jf = Vec3::ZERO;
            b.jt = Vec3::ZERO;
            b.push = Vec3::ZERO;
            b.turn = Vec3::ZERO;
        }
        out
    }

    /// What mcCarDamage multiplies the car's blow from `other` by (0x82342EA8): a vehicle's "damage
    /// inflicted" factor (its mcCarDamage +20; the car's own +16 is 1), anything else +12 = 1.
    pub fn vehicle_factor(&self, other: Option<Who>) -> f32 {
        match other {
            Some(Who::Obj(o)) => self.objs.get(o).and_then(|o| o.veh).map_or(1.0, |v| v.inflict),
            _ => 1.0,
        }
    }

    /// The other vehicles of the tick in hand, as the replay's file has them before it: each an
    /// object with the logged body; those not in the list are gone, and their contacts with them.
    fn sync_others(&mut self) {
        let Some(ot) = others() else { return };
        let tick = REPLAY_TICK.load(std::sync::atomic::Ordering::Relaxed);
        for o in self.objs.iter_mut() {
            if let Some(vh) = o.veh.as_mut() {
                vh.here = false;
                o.body = None;
            }
        }
        for v in ot.ticks.get(&tick).into_iter().flatten() {
            // (an inactive one is activated by its driver's code with a state no record holds: left out)
            if v.state != 0 {
                continue;
            }
            let Some(shape) = ot.shapes.get(&v.model) else { continue };
            let i = match self.objs.iter().position(|o| o.veh.map_or(false, |x| x.index == v.index)) {
                Some(i) => i,
                None => {
                    self.objs.push(Obj { ty: usize::MAX, inst: None, matrix: v.matrix, entry: None, hit: false, body: None, level: 0, addr: 0, veh: None, sleep: Sleep::reset(), dormant: false, amb: None });
                    self.objs.len() - 1
                }
            };
            // (its place among the level's indices and the heap's addresses, against the car's)
            let (car_level, car_addr) = self.car_place();
            let obj = &mut self.objs[i];
            obj.level = v.index as i64 - ot.mine as i64 + car_level;
            obj.addr = v.addr as i64 - ot.car_addr as i64 + car_addr;
            obj.matrix = v.matrix;
            obj.body = Some(v.body);
            obj.veh = Some(VehObj { shape, shape2: None, class: v.class, gravity: v.gravity, cg: v.cg, index: v.index, here: true, inflict: 1.0 });
        }
        let gone = |w: Who, objs: &Vec<Obj>| matches!(w, Who::Obj(o) if objs[o].veh.map_or(false, |x| !x.here || ot.inject));
        let objs = &self.objs;
        self.manis.retain(|m| !gone(m.a, objs) && !gone(m.b, objs));
        if ot.inject {
            for k in ot.kept.get(&tick).into_iter().flatten() {
                let Some(o) = self.objs.iter().position(|o| o.veh.map_or(false, |x| x.index == k.index && x.here)) else { continue };
                let vh = self.objs[o].veh.unwrap();
                let side = |local: Vec3| Side { body: None, point: Vec3::ZERO, local, rows: [Vec3::X, Vec3::Y, Vec3::Z], margin: 0.04, core: Vec3::ZERO };
                let t = Touch { depth: k.depth, a: side(k.local_a), b: side(k.local_b), normal: k.normal, back: k.back, kept: k.kept, ..Default::default() };
                let slot = Slot2 { t, age: 1, enabled: true, element: k.element, surface: vh.shape.surface_of(k.element_b), surface_a: Surface::Prop };
                match self.manis.iter().position(|m| m.a == Who::Car && m.b == Who::Obj(o)) {
                    Some(i) => self.manis[i].slots.push(slot),
                    None => self.manis.push(Mani { a: Who::Car, b: Who::Obj(o), part_a: 0, part_b: 0, cell: 0, stale: false, visited: true, island: 0, order: 0, slots: vec![slot], rope: false }),
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The ambient (traffic) cars in the level (PEOPLE_TRAFFIC.md "Hitting riding traffic", "Physical
// traffic"). Each is an mcAmbientCar whose instance (car +112) is in the level from its spawn:
// inactive while it rides (level state 1; its matrix moved each frame, sub_826FA760, and its
// velocity the place's change over the frame, inst +96, sub_826E9A40); activated by any contact
// (0x8258D9F0 -> 0x8258D3F8 -> slot 28 0x826FA810 -> vfunc 18 sub_826E7340 -> sub_826E96E0); then its
// mcAmbientCollider (car +2688) is the base phCollider with its own wheels (slot 14 sub_826FA0F8 ->
// sub_826FBA60) until the rest rule (sub_826F9EA8) takes it out again. A semi's cab and trailer
// are joined by three constraints while they are active (sub_826E7340, sub_82591FA0).

/// An ambient type's physics: `mcAmbientTypePhysicsTuning.xml` (mass, inertia box, gravity factor),
/// `mcAmbientWheelTuning.xml` (sub_826E5B30's derived numbers), the `.r155` bound and the skeleton's
/// wheel bones.
#[derive(Clone, Debug)]
pub struct AmbTune {
    /// the bound: one polyhedron, or a composite's two (a semi trailer's)
    pub shape: &'static HullShape,
    pub shape2: Option<&'static HullShape>,
    /// fMass x fMassScale; mass / 12 x (y^2 + z^2, x^2 + z^2, x^2 + y^2) of vInertialBox x
    /// fInertialBoxScale (sub_825A42C8), each at least a twentieth of the largest (sub_825A4088)
    pub mass: f32,
    pub inertia: Vec3,
    /// fGravityFactor (archetype +48)
    pub gravity: f32,
    /// bound +80 as the game sets it: (0, fCogOffset x centroid.y, 0)
    pub cg: Vec3,
    /// type +240: fCogSim x centroid.y
    pub cog_sim: f32,
    /// type +244 .. +256 (sub_826E5B30, Wq = mass x gravity factor x 9.8 / 4): spring Ts Wq / travel,
    /// its damping 2 Sd sqrt(Wq k), the tyre's 2 Wq Tt / St and 2 Td sqrt(Wq kt)
    pub k: f32,
    pub c: f32,
    pub kt: f32,
    pub ct: f32,
    /// fSuspensionTravel (+216), fTireStretch (+228), fTireGrip (+204)
    pub travel: f32,
    pub stretch: f32,
    pub grip: f32,
    /// the wheels (`iWheelCount`; the `wheel_i` bones, sub_82790E68; a trailer's 2 and 3 are its
    /// bones 0 and 1 with z turned)
    pub wheels: Vec<Vec3>,
    /// the hitch (sub_82790A18): a cab's or a trailer's
    pub hitch: Option<Vec3>,
}

/// A wheel's record (collider +676, 24 bytes): the spring's compression (+8), the tyre's stretch
/// (+12), the stretch as it was when it slid (+16) and the slide flag (+20).
#[derive(Clone, Copy, Debug, Default)]
pub struct AmbWheel {
    pub d: f32,
    pub s: f32,
    pub raw: f32,
    pub slide: bool,
}

/// An ambient car in the level.
#[derive(Clone, Debug)]
pub struct Amb {
    /// the session's car (vehicles.rs `uid`)
    pub uid: u32,
    pub tune: Arc<AmbTune>,
    pub wheels: [AmbWheel; 6],
    /// collider +864: the rest rule's count
    pub count: u32,
    /// inst +96: the place's change over the frame
    pub moved: Vec3,
    /// the collider's turn (+288), kept while inactive: zero from Init (sub_8259BC48)
    pub turn: Vec3,
    /// the semi's other car (+3604)
    pub partner: Option<u32>,
    /// made active / put to rest since `ambient_events` last looked
    pub woke: bool,
    pub slept: bool,
    last_at: Option<Vec3>,
    /// level byte bit 8 for this update's collide: the instance has +140 bit 0x80 (a race path's
    /// bPhysical, path +170 -> sub_826EAD00 -> sub_826FA588) and its matrix was moved this frame
    /// while inactive (sub_826FA760 -> sub_825880D8: the simulator's list, at most 120); the list
    /// is emptied after the collide (sub_8258E4D0 -> sub_82588220)
    pub posed: bool,
}

/// A constraint between two ambient cars (rage::phConstraint, max distance: sub_825B6D18): the
/// owner (the car that was activated, +3592 .. +3600), its two cars, a point in each one's frame
/// (+80 / +96) and the length (+160); +164 = 0.6 (sub_825B6030), +320 = 0, +328 = -1 (never
/// breaks), flags = 2 (sub_825B6278).
#[derive(Clone, Copy, Debug)]
pub struct Rope {
    pub owner: u32,
    pub a: u32,
    pub b: u32,
    pub la: Vec3,
    pub lb: Vec3,
    pub len: f32,
}

/// (diagnostics, `MCLA_DBG_ROPE`: the constraints and the ambient cars' activations)
fn rope_dbg() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var_os("MCLA_DBG_ROPE").is_some())
}

/// The first constraint's length: global 0x82909F6C, in the image's zero-filled tail and written by
/// nothing - 0 (a point constraint, kind 1).
const ROPE_POINT: f32 = 0.0;
/// +164: how deep one constraint's contact stands at most; the rest moves the bodies (sub_825B6750)
const ROPE_DEPTH: f32 = 0.6;

impl LooseState {
    fn amb_obj(&self, uid: u32) -> Option<usize> {
        self.objs.iter().position(|o| o.amb.as_ref().is_some_and(|a| a.uid == uid))
    }

    /// A riding (or otherwise kinematic) ambient car's instance where the game puts it this frame:
    /// into the level inactive at its first place; its velocity the place's change (inst +96).
    pub fn ambient_place(&mut self, uid: u32, tune: &Arc<AmbTune>, matrix: [Vec3; 4], partner: Option<u32>, inflict: f32, dt: f32) {
        let o = match self.amb_obj(uid) {
            Some(o) => o,
            None => {
                let veh = VehObj { shape: tune.shape, shape2: tune.shape2, class: 6, gravity: tune.gravity, cg: tune.cg, index: uid, here: true, inflict };
                let amb = Amb { uid, tune: tune.clone(), wheels: [AmbWheel::default(); 6], count: 0, moved: Vec3::ZERO, turn: Vec3::ZERO, partner, woke: false, slept: false, last_at: None, posed: false };
                // (its level index: above the stepping cars' 0 - the car is A against it)
                let level = 1_000_000 + uid as i64;
                let obj = Obj { ty: usize::MAX, inst: None, matrix, entry: None, hit: false, body: None, level, addr: level, veh: Some(veh), sleep: Sleep::reset(), dormant: false, amb: Some(amb) };
                // (a slot an ambient car left)
                match self.objs.iter().position(|o| o.amb.is_none() && o.veh.is_some_and(|v| v.index == u32::MAX)) {
                    Some(i) => {
                        self.objs[i] = obj;
                        i
                    }
                    None => {
                        self.objs.push(obj);
                        self.objs.len() - 1
                    }
                }
            }
        };
        if self.objs[o].body.is_some() {
            return;
        }
        self.objs[o].matrix = matrix;
        let a = self.objs[o].amb.as_mut().unwrap();
        a.partner = partner;
        a.moved = match a.last_at {
            Some(p) if dt > 0.0 => (matrix[3] - p) / dt,
            _ => Vec3::ZERO,
        };
        a.last_at = Some(matrix[3]);
    }

    /// A car whose instance has +140 bit 0x80 had its matrix moved this frame (sub_826FA760, after
    /// `ambient_place`): while it is inactive (level state 1) sub_825880D8 puts it on the
    /// simulator's list (at most 120) and sets its level byte bit 8 until the collide; the
    /// collision pass (sub_8258D9F0) then takes a pair of two inactive objects when either one has
    /// bit 8 - such a car meets riding / parked cars and knockable props while it is kinematic, and
    /// a contact activates both inactive sides (0x8258E07C -> 0x8258D3F8).
    pub fn ambient_posed(&mut self, uid: u32) {
        let Some(o) = self.amb_obj(uid) else { return };
        if self.objs[o].body.is_some() {
            return;
        }
        let listed = self.objs.iter().filter(|x| x.amb.as_ref().is_some_and(|a| a.posed)).count();
        if listed < 120 {
            self.objs[o].amb.as_mut().unwrap().posed = true;
        }
    }

    /// The ambient cars the session no longer has go from the level, with their pairs and constraints.
    pub fn ambient_keep(&mut self, alive: &dyn Fn(u32) -> bool) {
        let gone: Vec<usize> = (0..self.objs.len()).filter(|&o| self.objs[o].amb.as_ref().is_some_and(|a| !alive(a.uid))).collect();
        if gone.is_empty() {
            return;
        }
        for &o in &gone {
            let uid = self.objs[o].amb.as_ref().unwrap().uid;
            self.ropes.retain(|r| r.a != uid && r.b != uid);
            let obj = &mut self.objs[o];
            obj.amb = None;
            obj.body = None;
            if let Some(v) = obj.veh.as_mut() {
                v.here = false;
                v.index = u32::MAX;
            }
        }
        self.manis.retain(|m| ![m.a, m.b].iter().any(|w| matches!(w, Who::Obj(o) if gone.contains(o))));
    }

    /// The game makes an ambient car active (its footprint probe, an explosion, slots 16 / 17: the
    /// same 0x8258D3F8).
    pub fn ambient_let_go(&mut self, uid: u32) {
        if let Some(o) = self.amb_obj(uid) {
            self.ambient_activate(o);
        }
    }

    /// An active ambient car: its instance's matrix, velocity and turn.
    /// An ambient car's wheels' compression d (collider +676 + 24 i + 8), whether it is active or not
    /// (kept between activations; the rest rule's reset zeroes it).
    pub fn ambient_wheel_d(&self, uid: u32) -> Option<[f32; 6]> {
        let a = self.objs[self.amb_obj(uid)?].amb.as_ref()?;
        Some(std::array::from_fn(|i| a.wheels[i].d))
    }

    pub fn ambient_pose(&self, uid: u32) -> Option<([Vec3; 4], Vec3, Vec3)> {
        let o = &self.objs[self.amb_obj(uid)?];
        o.body.map(|b| (o.matrix, b.v, b.w))
    }

    /// What became of the ambient cars since the last look: (car, made active, put to rest).
    pub fn ambient_events(&mut self) -> Vec<(u32, bool, bool)> {
        let mut out = Vec::new();
        for o in self.objs.iter_mut() {
            if let Some(a) = o.amb.as_mut() {
                if a.woke || a.slept {
                    out.push((a.uid, a.woke, a.slept));
                    a.woke = false;
                    a.slept = false;
                }
            }
        }
        out
    }

    /// An inactive ambient car made active (0x8258D3F8 -> slot 28 0x826FA810, vfunc 41: its collider at
    /// the instance's velocity, inst +96; its turn the collider's own; the rest count from 0). A semi's
    /// cab or trailer then joins its partner by three constraints if the partner holds none, and makes
    /// the partner active (sub_826E7340 -> sub_826EA0A8).
    fn ambient_activate(&mut self, o: usize) {
        if self.objs[o].body.is_some() {
            return;
        }
        let Some(a) = self.objs[o].amb.as_ref() else { return };
        let t = a.tune.clone();
        let (uid, partner, moved, turn) = (a.uid, a.partner, a.moved, a.turn);
        let m = self.objs[o].matrix;
        let b = PBody::of_car(xf(&m, t.cg), [m[0], m[1], m[2]], moved, turn, t.mass, t.inertia, m, m, Vec3::ZERO, Vec3::ZERO);
        self.objs[o].body = Some(b);
        let a = self.objs[o].amb.as_mut().unwrap();
        a.count = 0;
        a.woke = true;
        if dbg() || rope_dbg() {
            eprintln!("AMBWAKE tick {} uid {uid} v {:?} hitch {:?} partner {:?} ropes {}", self.tick, moved, t.hitch, partner, self.ropes.len());
        }
        let (Some(h), Some(p)) = (t.hitch, partner) else { return };
        let Some(q) = self.amb_obj(p) else { return };
        if self.ropes.iter().any(|r| r.owner == p) {
            return;
        }
        let hp = self.objs[q].amb.as_ref().unwrap().tune.hitch.unwrap_or(Vec3::ZERO);
        // (the hitches; the points 10 m along each one's own +z and +y from them: 6 m and 1.5 m apart
        // at most - their z axes within 2 asin 0.3, their y axes within 2 asin 0.075)
        for (off, len) in [(Vec3::ZERO, ROPE_POINT), (Vec3::new(0.0, 0.0, 10.0), 6.0), (Vec3::new(0.0, 10.0, 0.0), 1.5)] {
            self.ropes.push(Rope { owner: uid, a: uid, b: p, la: h + off, lb: hp + off, len });
        }
        self.ambient_activate(q);
    }

    /// After the tick (car slot 6, sub_826E5AB8): an active ambient car's velocity is its place's
    /// change (sub_826E9A40), and the rest rule (sub_826F9EA8): from its fourth frame active, a frame
    /// whose velocity is within 0.001 m/s on every axis resets its wheels (sub_826FBA38) and takes it
    /// out (0x82588D90 -> slot 19 sub_826E68C0: its constraints go; the collider keeps its turn).
    pub fn ambient_frame(&mut self, dt: f32) {
        for o in 0..self.objs.len() {
            let Some(b) = self.objs[o].body else { continue };
            let at = self.objs[o].matrix[3];
            let Some(a) = self.objs[o].amb.as_mut() else { continue };
            a.moved = match a.last_at {
                Some(p) if dt > 0.0 => (at - p) / dt,
                _ => a.moved,
            };
            a.last_at = Some(at);
            if a.count > 2 {
                let still = |x: f32| x.abs() <= f32::from_bits(0x3a83_126f);
                if still(a.moved.x) && still(a.moved.y) && still(a.moved.z) {
                    a.wheels = [AmbWheel::default(); 6];
                    a.turn = b.w;
                    a.slept = true;
                    a.count = 0;
                    let uid = a.uid;
                    self.objs[o].body = None;
                    self.ropes.retain(|r| r.owner != uid);
                    if dbg() {
                        eprintln!("AMBREST tick {} uid {uid}", self.tick);
                    }
                }
            }
            if let Some(a) = self.objs[o].amb.as_mut() {
                a.count += 1;
            }
        }
    }

    /// The constraints' contacts for this update (0x82591210 -> 0x825B6F78), before the collide: each
    /// one's manifold made afresh. A side inactive is made active once the two points are more than
    /// the length and 0.01 apart, else nothing; a rope that is slack has no contact; past +164 the
    /// rest moves the two bodies apart by their masses (sub_825B6750, into the push).
    fn ropes_touch(&mut self) {
        self.manis.retain(|m| !m.rope);
        let ropes = self.ropes.clone();
        for r in ropes {
            let (Some(oa), Some(ob)) = (self.amb_obj(r.a), self.amb_obj(r.b)) else { continue };
            let (pa, pb) = (xf(&self.objs[oa].matrix, r.la), xf(&self.objs[ob].matrix, r.lb));
            let far = |pa: Vec3, pb: Vec3| {
                let d = pa - pb;
                let lim = r.len + 0.01;
                dot3(d, d) > lim * lim
            };
            if self.objs[oa].body.is_none() {
                if !far(pa, pb) {
                    continue;
                }
                self.ambient_activate(oa);
            }
            if self.objs[ob].body.is_none() {
                if !far(pa, pb) {
                    continue;
                }
                self.ambient_activate(ob);
            }
            // (0x825B6470)
            let d = pb - pa;
            let s2 = dot3(d, d);
            let (n, l) = if s2 > f32::from_bits(0x2b8c_bccc) {
                let l = s2.sqrt();
                let e = 1.0 / l;
                let e = e + e * (-(e * l - 1.0));
                (d * e, l)
            } else {
                (Vec3::Y, 0.0)
            };
            let err = l - r.len;
            if rope_dbg() {
                eprintln!("ROPE tick {} {} -> {} len {} err {:.4} active {} {} island-to-be", self.tick, r.a, r.b, r.len, err, self.objs[oa].body.is_some(), self.objs[ob].body.is_some());
            }
            if err < 0.0 {
                continue;
            }
            let kind = if r.len >= 0.01 { 2 } else { 1 };
            let (depth, excess) = if err > ROPE_DEPTH { (ROPE_DEPTH, err - ROPE_DEPTH) } else { (err, 0.0) };
            // (+320 = 0: the allowed depth on top)
            let depth = depth + solver::ALLOW;
            if excess > 0.0 {
                let (ia, ib) = (self.objs[oa].body.map_or(0.0, |b| b.inv_mass), self.objs[ob].body.map_or(0.0, |b| b.inv_mass));
                let (sa, sb) = (ia * ia, ib * ib);
                let (wa, wb) = if sa > sb {
                    let wa = 1.0 / ((sb / sa).sqrt() + 1.0);
                    (wa, 1.0 - wa)
                } else if sb > sa {
                    let wb = 1.0 / ((sa / sb).sqrt() + 1.0);
                    (1.0 - wb, wb)
                } else {
                    (0.5, 0.5)
                };
                if let Some(b) = self.objs[oa].body.as_mut() {
                    b.push = solver::direct(b.push, n * (wa * excess));
                }
                if let Some(b) = self.objs[ob].body.as_mut() {
                    b.push = solver::direct(b.push, n * (-(wb * excess)));
                }
            }
            let side = |m: &[Vec3; 4], p: Vec3| Side { body: None, point: p, local: inv(m, p - m[3]), rows: [m[0], m[1], m[2]], margin: 0.04, core: Vec3::ZERO };
            let t = Touch { depth, friction: 0.0, elasticity: 0.0, a: side(&self.objs[oa].matrix, pa), b: side(&self.objs[ob].matrix, pb), normal: n, back: Vec3::ZERO - n, kind, ..Default::default() };
            let slot = Slot2 { t, age: 0, enabled: true, element: 0, surface: Surface::Prop, surface_a: Surface::Prop };
            self.manis.push(Mani { a: Who::Obj(oa), b: Who::Obj(ob), part_a: 0, part_b: 0, cell: 0, stale: false, visited: true, island: 0, order: 0, slots: vec![slot], rope: true });
        }
    }
}

/// An active ambient car's wheels (slot 14 sub_826FA0F8 -> sub_826FBA60, per wheel): a probe from the
/// wheel's bone straight down the car to half a metre under its floor (sub_826E5540 -> sub_827974F0,
/// the ground; a polygon steeper than 45 degrees is no hit); the spring along the hit's normal with a
/// constant quarter of the car's weight on top, its push when it is past its travel; the tyre's
/// stretch across the normal against the slip - the front pair's whole, the others' less their rolling
/// - into the body's forces for the next update. `m` the instance matrix (rows X, up, Z, place).
fn ambient_wheels(a: &mut Amb, b: &mut PBody, m: &[Vec3; 4], city: &super::collision::WorldCollider, dt: f32) {
    if dt == 0.0 {
        return;
    }
    let t = a.tune.clone();
    let inv_dt = 1.0 / dt;
    let (x, u, z, p) = (m[0], m[1], m[2], m[3]);
    let yzx = |v: Vec3| Vec3::new(v.y, v.z, v.x);
    let zxy = |v: Vec3| Vec3::new(v.z, v.x, v.y);
    let gj = |a: Vec3, b: Vec3| zxy(a) * yzx(b) - yzx(a) * zxy(b);
    for (i, w) in t.wheels.iter().enumerate().take(6) {
        let rec = &mut a.wheels[i];
        let base = (x * w.x + p) + z * w.z;
        let top = base + u * w.y;
        let bottom = base - u * 0.5;
        let mid = (top + bottom) * 0.5;
        let seg = bottom - top;
        let len = seg.length();
        // (the city bound only, one-sided, the nearest hit: collision.rs `probe_front`)
        let hit = (len > 0.0).then(|| city.probe_front(top, seg / len, len)).flatten().map(|h| (h.point, h.normal)).filter(|(_, n)| !(n.y.abs() < f32::from_bits(0x3f35_0481)));
        let Some((contact, n)) = hit else {
            rec.slide = false;
            rec.d = ex::fnms(u.y, dt, rec.d);
            rec.d = rec.d.clamp(-t.travel, t.travel);
            continue;
        };
        // (slot 21: the point's velocity)
        let vp = b.v + cross(b.w, contact - b.at);
        let s = fma(b.v.z, b.v.z, fma(b.v.x, b.v.x, b.v.y * b.v.y)).sqrt();
        let slip = if i <= 1 {
            let wk = Vec3::new(dot3(z, Vec3::new(x.x, u.x, z.x)), dot3(z, Vec3::new(x.y, u.y, z.y)), dot3(z, Vec3::new(x.z, u.z, z.z)));
            let uu = zxy(wk) * yzx(vp) - yzx(wk) * zxy(vp);
            vp - wk * uu
        } else {
            z * s + vp
        };
        let c1 = gj(n, slip);
        let c2 = gj(n, c1);
        let diff = contact - mid;
        let d_old = rec.d;
        let d = fma(diff.x, u.x, fma(u.y, diff.y, u.z * diff.z));
        rec.d = d;
        let rate = if d_old == 0.0 { 0.0 } else { (d - d_old) * inv_dt };
        let cosn = fma(u.x, n.x, fma(u.z, n.z, u.y * n.y));
        let mut f = n * (cosn * fma(t.k, d, t.c * rate));
        // (sub_826E9658: a quarter of the archetype's weight, no gravity factor)
        let sup = (t.mass * f32::from_bits(0x411c_cccd)) * 0.25;
        f.y = sup + f.y;
        b.force = b.force + f;
        let r = u * (t.cog_sim * u.y) + (contact - p);
        b.torque = b.torque + gj(f, r);
        if d > t.travel {
            b.push = solver::direct(b.push, n * (d - t.travel));
        }
        let s_old = rec.s;
        let st = fma(c2.x, c2.x, fma(c2.y, c2.y, c2.z * c2.z)).sqrt() * dt;
        rec.s = st;
        if st > t.stretch {
            rec.raw = st;
            rec.slide = true;
            rec.s = t.grip * t.stretch;
        } else {
            rec.slide = false;
        }
        let rate_s = (rec.s - s_old) * inv_dt;
        let l2 = dot3(c2, c2);
        let dir = if l2 == 0.0 { Vec3::ZERO } else { c2 * super::gjk::rsqrt_as_original(l2) };
        let fac = if s < f32::from_bits(0x3f64_e26d) { 4.0 } else { 1.0 };
        let ft = ex::fms(fac * t.kt, rec.s, t.ct * rate_s) * u.y;
        let tyre = dir * ft;
        b.force = b.force + tyre;
        b.torque = b.torque + cross(contact - b.at, tyre);
        rec.d = rec.d.clamp(-t.travel, t.travel);
    }
}

// ---------------------------------------------------------------------------------------------
// A car's torn-off bumper (docs/reference/audio/DAMAGE_PARTS_RULE.md 2.4, 2.5): the anim object's
// mcCarAnimBreakablePartInst puts its own phInst into the level when both ends have broken
// (sub_823414C8 @823417FC -> 0x8258BE48). Its archetype and bound are made by the bumper's
// constructor (sub_8233C988): a phArchetypePhys (0x8258F538: max speed 500, max spin 2 pi) of mass 4,
// the angular inertia of a box 1.25 x the bound (0x825A42C8, floored by 0x825A4088), gravity factor
// 1.5 (vt+56 = 0x8258E6B0 writes archetype +48, the factor pre_collide multiplies gravity by), type /
// include flags 2 / 0xFFFFBFFF as a prop's; a box bound 1.8 x 0.333 x 0.5 (0x825807E0, full sizes,
// margin 0.04 from the bound's constructor 0x82583708), centre of gravity (0, 0, 0.15) (0x825832C8:
// bound +80, flag +6), material "prop" (0x82043524 through the material manager's vt+24, bound vt+52).
// No damping (a plain phArchetypePhys). It is a pool body with the simulator's sleep object (p1
// body_decode.md section 6), which the port's props do not have yet: its rule is kept here for the
// pieces alone.

/// The name the port gives the bumper's type among the level's (it is not in the prop file).
pub const PIECE_TYPE: &str = "mcCarAnimBreakablePartInst";

/// The bumper's type as the loose bodies read a type.
pub fn piece_type() -> FragType {
    let size = Vec3::new(1.8, 0.333, 0.5);
    let half = size * 0.5;
    let cg = [0.0, 0.0, 0.15];
    let radius = half.length();
    let mass = 4.0f32;
    // (0x825A42C8: ((y^2 + z^2) m) / 12 ... of the box 1.25 x the bound; 0x825A4088's floor leaves these as they are)
    let s = size * 1.25;
    let k = f32::from_bits(0x3daa_aaab);
    let inertia = [((s.z * s.z + s.y * s.y) * mass) * k, ((s.x * s.x + s.z * s.z) * mass) * k, ((s.x * s.x + s.y * s.y) * mass) * k];
    let corners: Vec<[f32; 3]> = (0..8).map(|i| [if i & 1 == 0 { -half.x } else { half.x }, if i & 2 == 0 { -half.y } else { half.y }, if i & 4 == 0 { -half.z } else { half.z }]).collect();
    let bound = FragBound { kind: 3, flag6: true, radius, max: half.to_array(), min: (-half).to_array(), centroid: [0.0; 3], cg, volume: size.x * size.y * size.z, margin: 0.04, verts: corners, ..Default::default() };
    FragType {
        off: usize::MAX,
        name: PIECE_TYPE.to_string(),
        // (a phInst of its own: no cache entry; activated again by any blow once it has gone to sleep)
        flags: 0,
        min_move: 0.0,
        elasticity: 0.5,
        archetype: mcla_formats::rage::fragphys::FragArchetype {
            type_flags: 2,
            include: 0xFFFF_BFFF,
            mass,
            inv_mass: 1.0 / mass,
            gravity: 1.5,
            max_speed: 500.0,
            max_spin: f32::from_bits(0x40c9_0fdb),
            inertia,
            inv_inertia: inertia.map(recip),
        },
        radius,
        max: half.to_array(),
        min: (-half).to_array(),
        centroid: [0.0; 3],
        cg,
        margin: 0.04,
        parts: vec![mcla_formats::rage::fragphys::FragPart { bound: Some(bound), matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0; 3]] }],
        ..Default::default()
    }
}

impl LooseWorld {
    /// The number of the torn bumper's type (appended by `new`).
    pub fn piece_type_index(&self) -> Option<usize> {
        self.types.iter().rposition(|t| t.off == usize::MAX && t.name == PIECE_TYPE)
    }
}

/// The simulator's sleep object of a pool body (0x82080724; body_decode.md 6.1): state 1 awake,
/// 0 settling, 2 asleep; the counters A and B against 15 and 20 ticks.
#[derive(Clone, Copy, Debug)]
pub struct Sleep {
    pub state: u8,
    pub a: u32,
    pub b: u32,
}

impl Sleep {
    /// 0x82594E80, at every activation: A = 15 - 3, B = 20 - 3, awake.
    pub fn reset() -> Sleep {
        Sleep { state: 1, a: 12, b: 17 }
    }
}

/// The "still" test (0x82595128 -> 0x82594EC0) with the game's tolerances: v^2 and w^2 0.05.
fn still(b: &PBody, dt: f32, f2: f32) -> bool {
    let [a, bb, c, d] = still_terms(b, dt, f2);
    let tol = f32::from_bits(0x3d4c_cccd);
    !(a > tol) && !(bb > tol) && !(c > tol) && !(d > tol)
}

/// The four terms the still test compares with its tolerances: |w1|^2, |v1|^2, and the growths by the tick's sums.
fn still_terms(b: &PBody, dt: f32, f2: f32) -> [f32; 4] {
    let invdt = if dt != 0.0 { 1.0 / dt } else { f32::MAX };
    let sq = |x: Vec3| fma(x.x, x.x, fma(x.y, x.y, x.z * x.z));
    // (+432 / +448 hold nothing the port has: zero)
    let w1 = b.turn * invdt + b.w;
    let a = sq(w1);
    let v1 = b.push * invdt + b.v;
    let bb = sq(v1);
    let x = b.torque * f2 + b.jt;
    let m = ex::world_inverse_inertia(&b.rows, b.inv_inertia);
    let w2 = Vec3::new(dot3(x, Vec3::new(m[0].x, m[1].x, m[2].x)), dot3(x, Vec3::new(m[0].y, m[1].y, m[2].y)), dot3(x, Vec3::new(m[0].z, m[1].z, m[2].z))) + w1;
    let c = sq(w2) - a;
    let y = b.force * f2 + b.jf;
    let v2 = y * b.inv_mass + v1;
    let d = sq(v2) - bb;
    [a, bb, c, d]
}

/// What the bumper's contact callback (sub_8234CF80) reads of one of its contacts this tick: every contact whose
/// +165 bit 0x80 is set - the contact's "enabled" bit (set with 0x40 when it is made, 0x825A26B8 / 0x825A2730;
/// cleared by a callback that takes the contact from the solver, `clrlwi ..,25` in 0x82332AC8, 0x8255D028 ...), which
/// is the port's `Slot2::enabled`; tested at 0x8234D028.
#[derive(Clone, Copy, Debug)]
pub struct PieceContact {
    /// the relative velocity at the contact (the piece less the other side), the normal as the
    /// callback turns it (negated when the piece is A of the pair), the place
    pub v: Vec3,
    pub n: Vec3,
    pub point: Vec3,
}

/// One torn-off bumper in the level.
#[derive(Clone, Debug)]
pub struct Piece {
    pub id: u64,
    pub obj: usize,
    pub sleep: Sleep,
    /// asleep and taken out of the simulator (0x82588D90): an inactive object at its last matrix
    pub off: bool,
    /// its contacts as the last refresh had them
    pub contacts: Vec<PieceContact>,
}

/// MCLA_DBG_PIECE: every enabled contact of a piece, tick by tick (stderr)
static DBG_PIECE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
static PIECE_IDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl LooseState {
    /// Put a torn bumper into the level at `matrix` (its frame: rows and place) moving at `v`, turning
    /// at `w` (0x8258BE48, then the body's vt+164 / vt+172). None where the level has no such type.
    pub fn add_piece(&mut self, world: &LooseWorld, matrix: [Vec3; 4], v: Vec3, w: Vec3) -> Option<u64> {
        let ty = world.piece_type_index()?;
        let t = &world.types[ty];
        let a = &t.archetype;
        let e = Entry { absent: 0, age: 0.0, cg: self::v(t.cg), mass: a.mass, inv_mass: a.inv_mass, inertia: self::v(a.inertia), inv_inertia: self::v(a.inv_inertia), ..Default::default() };
        let mut body = PBody::new(&matrix, e.cg, &e, a.max_speed, a.max_spin);
        body.v = v;
        body.p = v * body.mass;
        body.w = w;
        body.l = body.momentum_of(w);
        let o = self.objs.len();
        // (inserted after the car: the car is A against it)
        self.objs.push(Obj { ty, inst: None, matrix, entry: None, hit: true, body: Some(body), level: 3_000_000 + o as i64, addr: 3_000_000 + o as i64, veh: None, sleep: Sleep::reset(), dormant: false, amb: None });
        let id = PIECE_IDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.pieces.push(Piece { id, obj: o, sleep: Sleep::reset(), off: false, contacts: Vec::new() });
        Some(id)
    }

    /// Take a piece out of the level (the bumper's reset, sub_8233CF78). The object stays in the
    /// list (other objects are known by their place in it) far below the city, without a body.
    pub fn remove_piece(&mut self, id: u64) {
        let Some(k) = self.pieces.iter().position(|p| p.id == id) else { return };
        let o = self.pieces.remove(k).obj;
        let obj = &mut self.objs[o];
        obj.body = None;
        obj.hit = true;
        obj.matrix = [Vec3::X, Vec3::Y, Vec3::Z, Vec3::new(0.0, -100_000.0, 0.0)];
        self.manis.retain(|m| m.a != Who::Obj(o) && m.b != Who::Obj(o));
    }

    /// A piece's matrix as the level has it, its contacts of the last tick and whether it is asleep
    /// (None: not in this level).
    pub fn piece(&self, id: u64) -> Option<([Vec3; 4], &[PieceContact], bool)> {
        let p = self.pieces.iter().find(|p| p.id == id)?;
        Some((self.objs[p.obj].matrix, &p.contacts, p.off))
    }

    /// PreCollide's sleep test (0x82588490): a settling or asleep body that is not still wakes up.
    fn pieces_wake(&mut self, dt: f32) {
        for p in self.pieces.iter_mut() {
            let Some(b) = self.objs[p.obj].body.as_ref() else { continue };
            if p.off {
                // (hit while inactive: the breaking pass gave it a new body, v = w = 0)
                p.off = false;
                p.sleep = Sleep::reset();
                continue;
            }
            if p.sleep.state != 1 && !still(b, dt, 0.0) {
                p.sleep.state = 1;
            }
        }
    }

    /// The bumper's contact callback's view of its contacts after the refresh (sub_8233C938 -> sub_8234CF80).
    fn pieces_touched(&mut self, car: &CarIn) {
        for k in 0..self.pieces.len() {
            let o = self.pieces[k].obj;
            let mut out = Vec::new();
            if let Some(b) = self.objs[o].body {
                let vel_of = |w: Who, at: Vec3| match w {
                    Who::Obj(x) => self.objs[x].body.map_or(Vec3::ZERO, |ob| ob.v + cross(ob.w, at - ob.at)),
                    Who::Car => car.vel,
                    Who::City => Vec3::ZERO,
                };
                for m in self.manis.iter().filter(|m| m.a == Who::Obj(o) || m.b == Who::Obj(o)) {
                    let piece_is_a = m.a == Who::Obj(o);
                    let other = if piece_is_a { m.b } else { m.a };
                    for s in m.slots.iter().filter(|s| s.enabled) {
                        let at = if piece_is_a { s.t.a.point } else { s.t.b.point };
                        let v = (b.v + cross(b.w, at - b.at)) - vel_of(other, at);
                        let n = if piece_is_a { Vec3::ZERO - s.t.normal } else { s.t.normal };
                        out.push(PieceContact { v, n, point: at });
                        if *DBG_PIECE.get_or_init(|| std::env::var_os("MCLA_DBG_PIECE").is_some()) {
                            eprintln!("PIECECON tick {} other {:?} age {} depth {:.4} n {:?} v {:?} at {:?} body v {:?} w {:?}", self.tick, other, s.age, s.t.depth, n, v, at, b.v, b.w);
                        }
                    }
                }
            }
            self.pieces[k].contacts = out;
        }
    }

    /// The sleep object's update (0x825951D0, after the velocity solves, before the integration) and the
    /// deactivation of a piece asleep in an island of its own (0x825DAB28 -> 0x82588D90; the car and the
    /// port's props have no sleep object, so an island with any of them never sleeps).
    fn pieces_sleep(&mut self, dt: f32) {
        for k in 0..self.pieces.len() {
            let o = self.pieces[k].obj;
            let Some(b) = self.objs[o].body else { continue };
            if self.pieces[k].sleep.state == 2 {
                let alone = !self.manis.iter().any(|m| {
                    (m.a == Who::Obj(o) || m.b == Who::Obj(o)) && [m.a, m.b].into_iter().any(|w| w == Who::Car || matches!(w, Who::Obj(x) if x != o && self.objs[x].body.is_some()))
                });
                if alone {
                    self.pieces[k].off = true;
                    let obj = &mut self.objs[o];
                    obj.body = None;
                    // (an inactive instance the breaking pass may move again)
                    obj.hit = false;
                    continue;
                }
            }
            let s = &mut self.pieces[k].sleep;
            if still(&b, dt, dt) {
                if s.state == 1 {
                    s.a += 1;
                    if s.a >= 15 {
                        s.state = 0;
                    }
                }
                if s.state != 1 {
                    s.b += 1;
                    if s.b >= 20 {
                        s.state = 2;
                    }
                }
            } else if s.state == 1 {
                s.a = 0;
                s.b = 0;
            } else {
                s.state = 1;
            }
        }
    }
}

impl LooseState {
    /// Has the object a sleep object of the generic kind (a pool body that is no vehicle and no torn bumper)?
    fn sleeper(&self, o: usize) -> bool {
        // (an ambient car's collider has its own, collider +820 - sub_826F9FB8 -> 0x825952E0: the same rule)
        (self.objs[o].veh.is_none() || self.objs[o].amb.is_some()) && !self.pieces.iter().any(|p| p.obj == o)
    }

    /// Take an asleep object out of the simulator (0x82588D90): inactive at its last matrix, its entry and
    /// its "hit" bit kept.
    fn deactivate(&mut self, o: usize) {
        // (an ambient car: 0x82588D90 -> inst vtable[29] sub_826FA458 -> car slot 19 sub_826E68C0 - its
        // constraints go; the collider keeps its turn and its sleep object; the wheels are not reset)
        let body = self.objs[o].body;
        if let (Some(a), Some(b)) = (self.objs[o].amb.as_mut(), body) {
            a.turn = b.w;
            a.slept = true;
            let uid = a.uid;
            self.ropes.retain(|r| r.owner != uid);
            self.objs[o].body = None;
            if dbg() || rope_dbg() {
                eprintln!("AMBSLEEP tick {} uid {uid}", self.tick);
            }
            return;
        }
        let obj = &mut self.objs[o];
        obj.body = None;
        obj.dormant = true;
        if dbg() {
            eprintln!("LOOSESLEEP tick {} obj {o} level {}", self.tick, obj.level);
        }
    }

    /// 0x82590A60 -> 0x8258D3F8: an inactive object that has been hit is activated again with a new body at
    /// rest (0x8259E648: v = w = 0) and a reset sleep object; one never hit refuses (vtable[28] 0x8248BF08).
    fn wake_obj(&mut self, world: &LooseWorld, o: usize) {
        if !self.objs[o].dormant || self.objs[o].body.is_some() || !self.objs[o].hit || !self.sleeper(o) {
            return;
        }
        let t = &world.types[self.objs[o].ty];
        let obj = &mut self.objs[o];
        obj.body = Some(match obj.entry {
            Some(e) => PBody::new(&obj.matrix, e.cg, &e, 500.0, f32::from_bits(0x40c9_0fdb)),
            None => {
                let a = &t.archetype;
                let e = Entry { absent: 0, age: 0.0, cg: v(t.cg), mass: a.mass, inv_mass: a.inv_mass, inertia: v(a.inertia), inv_inertia: v(a.inv_inertia), ..Default::default() };
                PBody::new(&obj.matrix, e.cg, &e, a.max_speed, a.max_spin)
            }
        });
        (obj.sleep, obj.dormant) = (Sleep::reset(), false);
        if dbg() {
            eprintln!("LOOSEWAKE tick {} obj {o} level {}", self.tick, obj.level);
        }
    }

    /// PreCollide's sleep test (0x82588490) for every body with a sleep object.
    fn bodies_wake(&mut self, dt: f32) {
        for o in 0..self.objs.len() {
            if !self.sleeper(o) {
                continue;
            }
            let Some(b) = self.objs[o].body.as_ref() else { continue };
            if self.objs[o].sleep.state != 1 && !still(b, dt, 0.0) {
                self.objs[o].sleep.state = 1;
            }
        }
    }

    /// The sleep object's update (0x825951D0, in 0x8258A300's loop before the integration) and deactivation A
    /// (body_decode.md 6.2): asleep with no manifold this tick - not integrated, taken out.
    fn bodies_sleep(&mut self, dt: f32) {
        for o in 0..self.objs.len() {
            if !self.sleeper(o) {
                continue;
            }
            // (the Update's test runs before the push solve, and PreCollide's 0x8259E790 has left +432 / +448 zero:
            // no push or turn enters it - the port's are this tick's push solve's. V s_rtv 22265: not still at s385
            // by its w alone, asleep at s420 as the original, not s419)
            let Some(mut b) = self.objs[o].body else { continue };
            (b.push, b.turn) = (Vec3::ZERO, Vec3::ZERO);
            let s = &mut self.objs[o].sleep;
            if still(&b, dt, dt) {
                if s.state == 1 {
                    s.a += 1;
                    if s.a >= 15 {
                        s.state = 0;
                    }
                }
                if s.state != 1 {
                    s.b += 1;
                    if s.b >= 20 {
                        s.state = 2;
                    }
                }
            } else if s.state == 1 {
                s.a = 0;
                s.b = 0;
            } else {
                s.state = 1;
            }
            if dbg() {
                let s = &self.objs[o].sleep;
                eprintln!("LOOSESLEEPOBJ tick {} obj {o} state {} a {} b {} still {} terms {:?}", self.tick, s.state, s.a, s.b, still(&b, dt, dt), still_terms(&b, dt, dt));
            }
            if self.objs[o].sleep.state == 2 && !self.manis.iter().any(|m| m.a == Who::Obj(o) || m.b == Who::Obj(o)) {
                self.deactivate(o);
            }
        }
    }

    /// Deactivation B (0x825DAB28 -> 0x825906D0 / 0x82592A00): an island whose bodies are all asleep is taken
    /// out before its solve - no breaking, no impulses. A body without a sleep object of this kind (the car, a
    /// vehicle, a torn bumper - the port keeps the bumper's own rule) keeps its island awake.
    fn islands_asleep(&mut self) -> Vec<usize> {
        let mut seen: Vec<usize> = Vec::new();
        let mut asleep = Vec::new();
        for k in 0..self.manis.len() {
            let isl = self.manis[k].island;
            if isl == 0 || seen.contains(&isl) {
                continue;
            }
            seen.push(isl);
            let mut bodies: Vec<usize> = Vec::new();
            let mut all = true;
            for m in self.manis.iter().filter(|m| m.island == isl) {
                for w in [m.a, m.b] {
                    match w {
                        Who::Car => all = false,
                        Who::City => {}
                        Who::Obj(o) => {
                            if self.objs[o].body.is_none() {
                                continue;
                            }
                            if !self.sleeper(o) || self.objs[o].sleep.state != 2 {
                                all = false;
                            } else if !bodies.contains(&o) {
                                bodies.push(o);
                            }
                        }
                    }
                }
            }
            if all {
                asleep.push(isl);
                for o in bodies {
                    self.deactivate(o);
                }
            }
        }
        asleep
    }
}

/// A piece on its way from one car's level to another's (the original has one level: whichever car reaches the
/// piece meets it).
#[derive(Clone, Debug)]
pub struct PieceMove {
    id: u64,
    matrix: [Vec3; 4],
    body: Option<PBody>,
    hit: bool,
    sleep: Sleep,
    off: bool,
}

impl LooseState {
    /// Where a piece is and whether it touches this level's car now.
    pub fn piece_place(&self, id: u64) -> Option<(Vec3, bool)> {
        let p = self.pieces.iter().find(|p| p.id == id)?;
        let o = p.obj;
        let touching = self.manis.iter().any(|m| (m.a == Who::Obj(o) && m.b == Who::Car) || (m.b == Who::Obj(o) && m.a == Who::Car));
        Some((self.objs[o].matrix[3], touching))
    }

    /// The ids of the pieces this level holds.
    pub fn piece_ids(&self) -> Vec<u64> {
        self.pieces.iter().map(|p| p.id).collect()
    }

    /// Take a piece out of this level with its body and sleep state, for another car's.
    pub fn take_piece(&mut self, id: u64) -> Option<PieceMove> {
        let p = self.pieces.iter().find(|p| p.id == id)?.clone();
        let obj = &self.objs[p.obj];
        let out = PieceMove { id, matrix: obj.matrix, body: obj.body, hit: obj.hit, sleep: p.sleep, off: p.off };
        self.remove_piece(id);
        Some(out)
    }

    /// Put a piece taken from another car's level into this one, as it was.
    pub fn put_piece(&mut self, world: &LooseWorld, m: PieceMove) {
        let Some(ty) = world.piece_type_index() else { return };
        let o = self.objs.len();
        let mut body = m.body;
        if let Some(b) = body.as_mut() {
            // (the collide's "last" matrix is this level's from now on)
            b.last = m.matrix;
            b.last_set = true;
        }
        self.objs.push(Obj { ty, inst: None, matrix: m.matrix, entry: None, hit: m.hit, body, level: 3_000_000 + o as i64, addr: 3_000_000 + o as i64, veh: None, sleep: Sleep::reset(), dormant: false, amb: None });
        self.pieces.push(Piece { id: m.id, obj: o, sleep: m.sleep, off: m.off, contacts: Vec::new() });
    }
}

/// The cars' levels as the pieces see them: each car's place, how far it reaches, and its level.
pub struct CarLevel<'a> {
    pub at: Vec3,
    pub reach: f32,
    pub level: &'a mut LooseState,
}

/// Every piece goes to the level of the car that reaches it: a piece its own car neither touches nor reaches moves to
/// the nearest car that reaches it (two cars at one piece: the one that holds it keeps it).
pub fn hand_over_pieces(world: &LooseWorld, cars: &mut [CarLevel]) {
    for i in 0..cars.len() {
        for id in cars[i].level.piece_ids() {
            let Some((at, touching)) = cars[i].level.piece_place(id) else { continue };
            if touching || (at - cars[i].at).length() < cars[i].reach {
                continue;
            }
            let best = (0..cars.len()).filter(|&j| j != i).map(|j| (j, (at - cars[j].at).length())).filter(|&(j, d)| d < cars[j].reach).min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((j, _)) = best {
                if let Some(m) = cars[i].level.take_piece(id) {
                    cars[j].level.put_piece(world, m);
                }
            }
        }
    }
}

#[cfg(test)]
mod piece_tests {
    use super::*;
    use crate::physics::collision::{SectorMesh, WorldCollider};

    /// The bumper's type as sub_8233C988 makes it.
    #[test]
    fn the_torn_bumper_is_the_originals_box() {
        let t = piece_type();
        let a = &t.archetype;
        assert_eq!((a.mass, a.gravity, a.max_speed, a.type_flags, a.include), (4.0, 1.5, 500.0, 2, 0xFFFF_BFFF));
        // (the box 2.25 x 0.41625 x 0.625: (0.4163^2 + 0.625^2) 4 / 12 ...)
        assert!((a.inertia[0] - 0.187963).abs() < 1e-5 && (a.inertia[1] - 1.817708).abs() < 1e-5 && (a.inertia[2] - 1.745255).abs() < 1e-5, "{:?}", a.inertia);
        let b = t.parts[0].bound.as_ref().unwrap();
        assert_eq!((b.kind, b.max, b.margin, b.cg), (3, [0.9, 0.1665, 0.25], 0.04, [0.0, 0.0, 0.15]));
    }

    /// A piece its car has left goes, body and sleep state and all, to the level of the car that comes to it.
    #[test]
    fn a_piece_goes_to_the_car_that_reaches_it() {
        let world = LooseWorld::new(Vec::new(), Vec::new());
        let (mut a, mut b) = (LooseState::default(), LooseState::default());
        let id = a.add_piece(&world, [Vec3::X, Vec3::Y, Vec3::Z, Vec3::new(10.0, 0.3, 0.0)], Vec3::new(1.0, 0.0, 0.0), Vec3::ZERO).unwrap();
        {
            let mut cars = [CarLevel { at: Vec3::ZERO, reach: 4.0, level: &mut a }, CarLevel { at: Vec3::new(12.0, 0.0, 0.0), reach: 4.0, level: &mut b }];
            hand_over_pieces(&world, &mut cars);
        }
        assert!(a.piece(id).is_none());
        let (m, _, off) = b.piece(id).unwrap();
        assert!(m[3] == Vec3::new(10.0, 0.3, 0.0) && !off);
        assert_eq!(b.objs.last().unwrap().body.unwrap().v, Vec3::new(1.0, 0.0, 0.0));
        // (the car that holds it and reaches it keeps it)
        let mut cars = [CarLevel { at: Vec3::new(9.0, 0.0, 0.0), reach: 4.0, level: &mut a }, CarLevel { at: Vec3::new(12.0, 0.0, 0.0), reach: 4.0, level: &mut b }];
        hand_over_pieces(&world, &mut cars);
        assert!(b.piece(id).is_some());
    }

    fn flat() -> WorldCollider {
        let mut w = WorldCollider::default();
        let (a, b, c, d) = (Vec3::new(-20.0, 0.0, -20.0), Vec3::new(-20.0, 0.0, 20.0), Vec3::new(20.0, 0.0, -20.0), Vec3::new(20.0, 0.0, 20.0));
        w.sectors.insert(1, SectorMesh::of_bound(vec![[a, b, c], [c, b, d]], vec![Surface::Road as u8, Surface::Road as u8], &[800.0, 800.0], vec![[None, None, None]; 2]));
        w
    }

    /// Dropped from half a metre, the piece comes to rest on the face it lands on, 0.25 m (standing on its 1.8 x
    /// 0.333 face) or 0.1665 m (lying on 1.8 x 0.5) above the ground, and is put to sleep (taken out of the
    /// simulator) about a second later.
    #[test]
    fn a_dropped_piece_lies_still_and_sleeps() {
        let world = LooseWorld::new(Vec::new(), Vec::new());
        let city = flat();
        let hull = HullShape::new(vec![Vec3::new(-1.0, -0.5, -2.0), Vec3::new(1.0, -0.5, -2.0), Vec3::new(0.0, 0.5, -2.0), Vec3::new(0.0, -0.5, 2.0)], &[[0, 1, 2], [0, 3, 1], [1, 3, 2], [2, 3, 0]]);
        let away = [Vec3::X, Vec3::Y, Vec3::Z, Vec3::new(1000.0, 0.0, 1000.0)];
        let car = CarIn { hull: &hull, hull_cg: Vec3::ZERO, hull_margin: 0.04, now: away, last: away, speed: 0.0, bound: (hull.min, hull.max), wheels: Vec::new(), vel: Vec3::ZERO, touch: (0.5, 0.1), wiped: false };
        for (rows, height) in [([Vec3::X, Vec3::Z, Vec3::NEG_Y], 0.25f32), ([Vec3::X, Vec3::Y, Vec3::Z], 0.1665)] {
            let mut s = LooseState::default();
            let id = s.add_piece(&world, [rows[0], rows[1], rows[2], Vec3::new(0.0, height + 0.5, 0.0)], Vec3::ZERO, Vec3::ZERO).unwrap();
            let dt = 1.0 / 60.0;
            let mut asleep = None;
            for k in 0..600 {
                s.pre_collide(&world, dt);
                s.collide(&world, &car, &city, false);
                s.refresh(&world, &car);
                s.step_rest(&world, &car, dt);
                s.end_tick(&world, dt);
                if asleep.is_none() && s.piece(id).unwrap().2 {
                    asleep = Some(k);
                }
            }
            let (m, _, off) = s.piece(id).unwrap();
            println!("rest {:?} up rows {:?} asleep at tick {:?}", m[3], (m[0].y, m[1].y, m[2].y), asleep);
            assert!(off && (m[3].y - height).abs() < 0.01, "{:?}", m);
        }
    }
}
